#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AppCommand {
    RunGateway,
    Doctor,
    Status { deep: bool },
    Onboard { force: bool },
}

pub fn parse_app_command(args: &[String]) -> Result<AppCommand, String> {
    match args.first().map(|arg| arg.as_str()) {
        None => Ok(AppCommand::RunGateway),
        Some("run") => {
            if args.len() == 1 {
                Ok(AppCommand::RunGateway)
            } else {
                Err("run does not accept extra arguments".to_string())
            }
        }
        Some("doctor") => {
            if args.len() == 1 {
                Ok(AppCommand::Doctor)
            } else {
                Err("doctor does not accept extra arguments".to_string())
            }
        }
        Some("status") => match args.get(1).map(|arg| arg.as_str()) {
            None => Ok(AppCommand::Status { deep: false }),
            Some("--deep") if args.len() == 2 => Ok(AppCommand::Status { deep: true }),
            Some(other) => Err(format!("unsupported status flag: {other}")),
        },
        Some("onboard") => match args.get(1).map(|arg| arg.as_str()) {
            None => Ok(AppCommand::Onboard { force: false }),
            Some("--force") if args.len() == 2 => Ok(AppCommand::Onboard { force: true }),
            Some(other) => Err(format!("unsupported onboard flag: {other}")),
        },
        Some(other) => Err(format!("unsupported command: {other}")),
    }
}

#[cfg(test)]
mod tests {
    use super::{parse_app_command, AppCommand};

    fn vec_of(args: &[&str]) -> Vec<String> {
        args.iter().map(|arg| arg.to_string()).collect()
    }

    #[test]
    fn defaults_to_run_gateway_when_no_args_are_present() {
        assert_eq!(parse_app_command(&[]), Ok(AppCommand::RunGateway));
    }

    #[test]
    fn parses_doctor_command() {
        assert_eq!(
            parse_app_command(&vec_of(&["doctor"])),
            Ok(AppCommand::Doctor)
        );
    }

    #[test]
    fn parses_status_with_deep_flag() {
        assert_eq!(
            parse_app_command(&vec_of(&["status", "--deep"])),
            Ok(AppCommand::Status { deep: true })
        );
    }

    #[test]
    fn parses_onboard_force_flag() {
        assert_eq!(
            parse_app_command(&vec_of(&["onboard", "--force"])),
            Ok(AppCommand::Onboard { force: true })
        );
    }
}
