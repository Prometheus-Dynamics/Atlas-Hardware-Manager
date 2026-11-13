/// The address of a robot's NT4 server from its team number: team 5338 is `10.53.38.2`.
/// `None` for a number that is not a team (0, or above 25599).
pub fn team_host(team: u32) -> Option<String> {
    (1..=25_599)
        .contains(&team)
        .then(|| format!("10.{}.{}.2", team / 100, team % 100))
}

#[cfg(test)]
mod tests {
    use super::team_host;

    #[test]
    fn a_team_number_is_its_robot_address() {
        assert_eq!(team_host(5338).as_deref(), Some("10.53.38.2"));
        assert_eq!(team_host(1).as_deref(), Some("10.0.1.2"));
        assert_eq!(team_host(25_599).as_deref(), Some("10.255.99.2"));
        assert_eq!(team_host(0), None);
        assert_eq!(team_host(25_600), None);
    }
}
