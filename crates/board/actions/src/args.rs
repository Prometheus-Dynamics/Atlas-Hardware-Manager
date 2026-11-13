//! Arguments: words on the way to the board, checked against the board's description where it
//! gives one.
//!
//! The board takes arguments as UTF-8 words separated by spaces, which Lemnos's bridge turns into
//! typed values. A word cannot hold a space, so a word with one is refused here, not sent. Where
//! the description says the types, each word is checked against its type before the call; where
//! it does not, the words go as they are.

use crate::catalog::{Action, ArgKind, Reported};
use crate::error::ActionError;

/// Checks `words` against `action`'s arguments.
///
/// # Errors
///
/// [`ActionError::BadArguments`] naming the first word that does not fit, or the count that is
/// wrong. Nothing is sent when this fails.
pub(crate) fn check(action: &Action, words: &[String]) -> Result<(), ActionError> {
    for word in words {
        if word.is_empty() || word.contains(char::is_whitespace) {
            return Err(ActionError::BadArguments(format!(
                "{word:?} is not one word: an argument holds no spaces and is not empty"
            )));
        }
    }
    let Reported::Known(args) = &action.args else {
        return Ok(());
    };
    if args.len() != words.len() {
        let wanted: Vec<String> = args
            .iter()
            .map(|arg| format!("{} ({})", arg.name, kind_text(&arg.kind)))
            .collect();
        return Err(ActionError::BadArguments(format!(
            "{} takes {} argument(s): {}; {} given",
            action.name,
            args.len(),
            if wanted.is_empty() {
                "none".to_owned()
            } else {
                wanted.join(", ")
            },
            words.len()
        )));
    }
    for (arg, word) in args.iter().zip(words) {
        let fits = match &arg.kind {
            ArgKind::Int => word.parse::<i64>().is_ok(),
            ArgKind::Float => word.parse::<f64>().is_ok_and(f64::is_finite),
            ArgKind::Bool => matches!(word.as_str(), "true" | "false" | "on" | "off" | "1" | "0"),
            ArgKind::Text | ArgKind::Other(_) => true,
        };
        if !fits {
            return Err(ActionError::BadArguments(format!(
                "{} is {}, and {word:?} is not",
                arg.name,
                kind_text(&arg.kind)
            )));
        }
    }
    Ok(())
}

/// The words as the wire carries them: one space between them.
pub(crate) fn encode(words: &[String]) -> Vec<u8> {
    words.join(" ").into_bytes()
}

fn kind_text(kind: &ArgKind) -> String {
    match kind {
        ArgKind::Int => "an int".into(),
        ArgKind::Float => "a float".into(),
        ArgKind::Bool => "a bool".into(),
        ArgKind::Text => "text".into(),
        ArgKind::Other(name) => format!("a {name}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::catalog::{Arg, Route};

    fn action(args: Reported<Vec<Arg>>) -> Action {
        Action {
            name: "vendor.dry-run".into(),
            label: Reported::NotReported,
            description: Reported::NotReported,
            args,
            long_running: Reported::Known(false),
        }
    }

    fn words(text: &str) -> Vec<String> {
        text.split_whitespace().map(str::to_owned).collect()
    }

    #[test]
    fn typed_words_are_checked_and_a_bad_one_is_named() {
        let typed = action(Reported::Known(vec![
            Arg {
                name: "label".into(),
                kind: ArgKind::Text,
            },
            Arg {
                name: "steps".into(),
                kind: ArgKind::Int,
            },
        ]));
        assert!(check(&typed, &words("run 3")).is_ok());
        let error = check(&typed, &words("run many")).unwrap_err();
        assert!(
            matches!(&error, ActionError::BadArguments(text) if text.contains("steps") && text.contains("many")),
            "{error}"
        );
        let count = check(&typed, &words("run")).unwrap_err();
        assert!(
            matches!(&count, ActionError::BadArguments(text) if text.contains("takes 2")),
            "{count}"
        );
        assert_eq!(typed.route(), Route::Direct);
    }

    #[test]
    fn words_the_board_does_not_describe_go_unchecked() {
        let open = action(Reported::NotReported);
        assert!(check(&open, &words("anything at all 7")).is_ok());
        assert!(
            check(&open, &[String::new()]).is_err(),
            "an empty word is never sent"
        );
    }

    #[test]
    fn a_word_with_a_space_is_refused_before_it_is_sent() {
        let open = action(Reported::NotReported);
        assert!(check(&open, &["two words".to_owned()]).is_err());
    }

    #[test]
    fn bool_words_take_the_board_spellings() {
        let flag = action(Reported::Known(vec![Arg {
            name: "on".into(),
            kind: ArgKind::Bool,
        }]));
        for ok in ["true", "off", "1"] {
            assert!(check(&flag, &words(ok)).is_ok(), "{ok}");
        }
        assert!(check(&flag, &words("yes")).is_err());
        assert_eq!(encode(&words("a b")), b"a b");
    }
}
