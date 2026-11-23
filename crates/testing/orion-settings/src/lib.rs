//! Every setting an image gives Orion is one the Orion it pins reads.
//!
//! Orion only warns about a key or an `ORION_*` variable it does not know. A setting written for
//! a newer Orion than the image pins is then dropped on the board without a word anyone sees, and
//! what it was meant to restrict is not restricted (a `peer.interface` list an older Orion never
//! reads leaves `peer.listen = 0.0.0.0` open on every interface). So the settings are read here,
//! off the board, by the pinned Orion's own code, and anything it would only warn about is a
//! failure by name.
//!
//! What is read, per image of a product that installs `/usr/bin/orion`:
//!
//! - the file installed as `/etc/orion/orion.conf`;
//! - every `ORION_*` assignment of an `Environment=` line in a file installed as the
//!   `orion-comm.service` or `orion-node.service` unit or one of its drop-ins, from the
//!   product's own files. Orion's own unit files come from the pinned source and are its own to keep;
//! - the `ORION_*` names written to a file one of those drop-ins reads with `EnvironmentFile=`:
//!   the installed files that name that file are searched for them. Their values exist only on
//!   the board, so only the names are checked.
//!
//! The rule for "unknown" is Orion's (`crates/bins/orion/src/lib.rs`, `unread_by_both`): the comm
//! core and node logic read one file, so a key is unknown only when neither reads it. A value that
//! does not parse is a failure too. `ORION_LOG`, `ORION_RECOVERY_KEYS` and `ORION_RECOVERY_LISTEN`
//! are not settings of either service and are left out, as Orion leaves them out.

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use orion_comm_app::CommSettings;
use orion_config::{Source, Warning};
use orion_node_app::NodeSettings;

/// The Orion commit this crate links (its `Cargo.toml`): the settings code that does the reading.
pub const LINKED: &str = "b01a7fb2fc76436b551b8f61ac8ebe3d332164ab";

const CONF: &str = "/etc/orion/orion.conf";
const BINARY: &str = "/usr/bin/orion";
const NOT_SETTINGS: [&str; 3] = ["ORION_LOG", "ORION_RECOVERY_KEYS", "ORION_RECOVERY_LISTEN"];

/// One of the two services that read the settings.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Service {
    /// `orion-comm.service`.
    Comm,
    /// `orion-node.service`.
    Node,
}

impl Service {
    const fn unit(self) -> &'static str {
        match self {
            Self::Comm => "orion-comm.service",
            Self::Node => "orion-node.service",
        }
    }
}

/// One `ORION_*` variable an image gives a service.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Variable {
    /// Its name.
    pub name: String,
    /// Its value, when the image states it; `None` for one written on the board.
    pub value: Option<String>,
    /// The file it comes from, relative to `devices/`.
    pub from: String,
}

/// What one image gives Orion.
#[derive(Clone, Debug, Default)]
pub struct Image {
    /// `<product>/<image>`.
    pub name: String,
    /// The Orion commit the image's `orion` binary is built from.
    pub pinned: String,
    /// The settings file: where it comes from and its text. `None` when the image makes it at run time.
    pub conf: Option<(String, String)>,
    /// The variables of each service's unit and drop-ins, in drop-in order.
    pub variables: BTreeMap<Service, Vec<Variable>>,
}

fn table(path: &Path) -> Result<toml::Table, String> {
    let text = fs::read_to_string(path).map_err(|error| format!("{}: {error}", path.display()))?;
    text.parse::<toml::Table>()
        .map_err(|error| format!("{}: {error}", path.display()))
}

/// `path` as read from `devices/`, with its `..` steps taken.
fn relative(devices: &Path, path: &Path) -> String {
    let mut clean = PathBuf::new();
    for part in path.strip_prefix(devices).unwrap_or(path).components() {
        match part {
            std::path::Component::ParentDir => {
                clean.pop();
            }
            other => clean.push(other),
        }
    }
    clean.display().to_string()
}

/// The assignments of one `Environment=` value, split as systemd splits it: at white space, with
/// `"…"` and `'…'` keeping a value with a space whole.
#[must_use]
pub fn assignments(value: &str) -> Vec<String> {
    let mut words = Vec::new();
    let mut word = String::new();
    let mut quote = None;
    let mut any = false;
    for ch in value.chars() {
        match (quote, ch) {
            (Some(open), _) if ch == open => quote = None,
            (Some(_), _) => word.push(ch),
            (None, '"' | '\'') => {
                quote = Some(ch);
                any = true;
            }
            (None, _) if ch.is_whitespace() => {
                if any || !word.is_empty() {
                    words.push(std::mem::take(&mut word));
                }
                any = false;
            }
            (None, _) => word.push(ch),
        }
    }
    if any || !word.is_empty() {
        words.push(word);
    }
    words
}

fn is_name(name: &str) -> bool {
    name.strip_prefix("ORION_").is_some_and(|rest| {
        !rest.is_empty()
            && rest
                .chars()
                .all(|ch| ch.is_ascii_uppercase() || ch.is_ascii_digit() || ch == '_')
    })
}

/// The `ORION_*` names assigned in `text` (`ORION_X=`), where the name is not the tail of a longer one.
fn names_assigned(text: &str) -> Vec<String> {
    let mut names = Vec::new();
    for (at, _) in text.match_indices("ORION_") {
        let before = text[..at].chars().next_back();
        if before.is_some_and(|ch| ch.is_ascii_alphanumeric() || ch == '_') {
            continue;
        }
        let rest = &text[at..];
        let end = rest
            .find(|ch: char| !(ch.is_ascii_uppercase() || ch.is_ascii_digit() || ch == '_'))
            .unwrap_or(rest.len());
        if rest[end..].starts_with('=')
            && is_name(&rest[..end])
            && !names.contains(&rest[..end].to_owned())
        {
            names.push(rest[..end].to_owned());
        }
    }
    names
}

/// The service a file installed at `to` belongs to, as its unit or one of its drop-ins.
fn service_of(to: &str) -> Option<Service> {
    let rest = to
        .strip_prefix("/etc/systemd/system/")
        .or_else(|| to.strip_prefix("/usr/lib/systemd/system/"))?;
    [Service::Comm, Service::Node].into_iter().find(|service| {
        rest == service.unit()
            || rest
                .strip_prefix(service.unit())
                .is_some_and(|tail| tail.starts_with(".d/") && tail.ends_with(".conf"))
    })
}

/// Every image under `devices/products` that installs the `orion` binary, with what it gives Orion.
///
/// # Errors
///
/// A description that cannot be read.
pub fn images(devices: &Path) -> Result<Vec<Image>, String> {
    let mut found = Vec::new();
    let products = devices.join("products");
    let mut dirs: Vec<PathBuf> = fs::read_dir(&products)
        .map_err(|error| format!("{}: {error}", products.display()))?
        .filter_map(|entry| entry.ok().map(|entry| entry.path()))
        .filter(|path| path.join("product.toml").is_file())
        .collect();
    dirs.sort();
    for dir in dirs {
        let product = table(&dir.join("product.toml"))?;
        let components = product.get("components").and_then(toml::Value::as_table);
        let Some(images) = product.get("images").and_then(toml::Value::as_array) else {
            continue;
        };
        for image in images {
            let mut out = Image {
                name: format!(
                    "{}/{}",
                    dir.file_name().unwrap_or_default().to_string_lossy(),
                    image
                        .get("name")
                        .and_then(toml::Value::as_str)
                        .unwrap_or("?")
                ),
                ..Image::default()
            };
            // (drop-in file name, service, variables): sorted by name afterwards, as systemd applies them.
            let mut units: Vec<(String, Service, Vec<Variable>)> = Vec::new();
            // Every local file the image installs, for the search of what writes an EnvironmentFile.
            let mut local: Vec<(String, String)> = Vec::new();
            let mut env_files: Vec<(Service, String)> = Vec::new();
            let mut has_orion = false;
            let names = image.get("components").and_then(toml::Value::as_array);
            for name in names.into_iter().flatten().filter_map(toml::Value::as_str) {
                let Some(rel) = components
                    .and_then(|c| c.get(name))
                    .and_then(toml::Value::as_str)
                else {
                    continue;
                };
                let path = dir.join(rel);
                let component = table(&path)?;
                let base = path.parent().unwrap_or(&dir).to_path_buf();
                let installs = component.get("install").and_then(toml::Value::as_array);
                for install in installs.into_iter().flatten() {
                    let to = install
                        .get("to")
                        .and_then(toml::Value::as_str)
                        .unwrap_or("");
                    if to == BINARY {
                        has_orion = true;
                        out.pinned = orion_ref(&component).unwrap_or_default();
                    }
                    let Some(file) = install
                        .get("from")
                        .and_then(|from| from.get("file"))
                        .and_then(toml::Value::as_str)
                    else {
                        continue;
                    };
                    let source = base.join(file);
                    let Ok(text) = fs::read_to_string(&source) else {
                        continue; // not text: a binary file gives no setting
                    };
                    let from = relative(devices, &source);
                    if to == CONF {
                        out.conf = Some((from.clone(), text.clone()));
                    }
                    if let Some(service) = service_of(to) {
                        let mut variables = Vec::new();
                        for line in text.lines() {
                            let line = line.trim();
                            if let Some(value) = line.strip_prefix("Environment=") {
                                for word in assignments(value) {
                                    if let Some((name, value)) = word.split_once('=')
                                        && is_name(name)
                                    {
                                        variables.push(Variable {
                                            name: name.to_owned(),
                                            value: Some(value.to_owned()),
                                            from: from.clone(),
                                        });
                                    }
                                }
                            } else if let Some(file) = line.strip_prefix("EnvironmentFile=") {
                                env_files.push((service, file.trim_start_matches('-').to_owned()));
                            }
                        }
                        let order = to.rsplit('/').next().unwrap_or(to).to_owned();
                        units.push((order, service, variables));
                    }
                    local.push((from, text));
                }
            }
            if !has_orion {
                continue;
            }
            units.sort_by(|a, b| a.0.cmp(&b.0));
            for (_, service, variables) in units {
                out.variables.entry(service).or_default().extend(variables);
            }
            // What is written on the board into a file a drop-in reads: the names, from the files that name it.
            for (service, env_file) in env_files {
                let base = env_file.rsplit('/').next().unwrap_or(&env_file);
                for (from, text) in &local {
                    if service_of_text_is_unit(text) || !text.contains(base) {
                        continue;
                    }
                    for name in names_assigned(text) {
                        let list = out.variables.entry(service).or_default();
                        if !list.iter().any(|known| known.name == name) {
                            list.push(Variable {
                                name,
                                value: None,
                                from: from.clone(),
                            });
                        }
                    }
                }
            }
            found.push(out);
        }
    }
    Ok(found)
}

/// Whether `text` is a unit or drop-in (its `EnvironmentFile=` line names the file, it does not write it).
fn service_of_text_is_unit(text: &str) -> bool {
    text.lines()
        .any(|line| line.trim_start().starts_with("EnvironmentFile="))
}

/// The commit of the component's Orion git source.
fn orion_ref(component: &toml::Table) -> Option<String> {
    let sources = component.get("sources")?.as_table()?;
    sources.values().find_map(|source| {
        let git = source.get("git")?.as_str()?;
        git.to_ascii_lowercase()
            .trim_end_matches(".git")
            .ends_with("/orion")
            .then(|| {
                source
                    .get("ref")
                    .and_then(toml::Value::as_str)
                    .map(str::to_owned)
            })?
    })
}

fn source(text: &str, variables: &[(String, String)]) -> Result<Source, String> {
    Source::parse(
        text,
        variables
            .iter()
            .filter(|(name, _)| !NOT_SETTINGS.contains(&name.as_str()))
            .cloned(),
    )
    .map_err(|error| error.to_string())
}

/// What `service` leaves unread of `text` and `variables`, or why its settings do not parse.
fn unread(
    service: Service,
    text: &str,
    variables: &[(String, String)],
) -> Result<Vec<Warning>, String> {
    let mut source = source(text, variables)?;
    match service {
        Service::Comm => drop(CommSettings::from_source(&mut source).map_err(|e| e.to_string())?),
        Service::Node => drop(NodeSettings::from_source(&mut source).map_err(|e| e.to_string())?),
    }
    Ok(source.warnings())
}

/// What neither service reads of `text` and `variables`, as `service` would report it (Orion's
/// `unread_by_both`), or why `service` cannot read its settings at all.
fn unknown(
    service: Service,
    text: &str,
    variables: &[(String, String)],
) -> Result<Vec<Warning>, String> {
    let mine = unread(service, text, variables)?;
    let other = match service {
        Service::Comm => Service::Node,
        Service::Node => Service::Comm,
    };
    Ok(match unread(other, text, variables) {
        Ok(theirs) => mine.into_iter().filter(|w| theirs.contains(w)).collect(),
        Err(_) => mine,
    })
}

/// The failures of one image: each setting the pinned Orion would not read, or cannot parse, by name.
#[must_use]
pub fn check(image: &Image) -> Vec<String> {
    let mut problems = Vec::new();
    if image.pinned != LINKED {
        problems.push(format!(
            "{}: its orion is built from Orion {}, and this check reads settings with Orion {LINKED}: nothing was checked. Move the two together (crates/testing/orion-settings/Cargo.toml)",
            image.name, image.pinned
        ));
        return problems;
    }
    let Some((conf_from, conf)) = &image.conf else {
        problems.push(format!(
            "{}: no file is installed as {CONF}, so its settings cannot be read here",
            image.name
        ));
        return problems;
    };
    for service in [Service::Comm, Service::Node] {
        let given = image.variables.get(&service).map_or(&[][..], Vec::as_slice);
        // The stated ones: the last assignment of a name wins, as in the unit.
        let mut stated: BTreeMap<String, (String, String)> = BTreeMap::new();
        for variable in given {
            if let Some(value) = &variable.value {
                stated.insert(
                    variable.name.clone(),
                    (value.clone(), variable.from.clone()),
                );
            }
        }
        let pairs: Vec<(String, String)> = stated
            .iter()
            .map(|(name, (value, _))| (name.clone(), value.clone()))
            .collect();
        match unknown(service, conf, &pairs) {
            Err(error) => problems.push(format!(
                "{}: {} cannot read its settings ({conf_from} and its unit's variables): {error}",
                image.name,
                service.unit()
            )),
            Ok(warnings) => {
                for warning in warnings {
                    let from = stated
                        .get(&warning.key)
                        .map_or(conf_from.as_str(), |(_, from)| from.as_str());
                    problems.push(format!(
                        "{}: Orion {} does not read `{}` ({from}; {}): the board would drop it with only a warning",
                        image.name,
                        &LINKED[..8],
                        warning.key,
                        service.unit()
                    ));
                }
            }
        }
        // The ones written on the board: the name alone, tried with a value that parses as nothing.
        for variable in given.iter().filter(|variable| variable.value.is_none()) {
            if stated.contains_key(&variable.name) || NOT_SETTINGS.contains(&variable.name.as_str())
            {
                continue;
            }
            let mut tried = pairs.clone();
            tried.push((variable.name.clone(), "\u{1}".to_owned()));
            // A value Orion refuses is a name it reads; only a name left unread by both is unknown.
            if let Ok(warnings) = unknown(service, conf, &tried)
                && warnings.iter().any(|warning| warning.key == variable.name)
            {
                problems.push(format!(
                    "{}: Orion {} does not read `{}` (written on the board; named in {}; {}): the board would drop it with only a warning",
                    image.name,
                    &LINKED[..8],
                    variable.name,
                    variable.from,
                    service.unit()
                ));
            }
        }
    }
    problems
}
