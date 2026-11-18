use crate::code::installs;

const DATA_CHANGING_WORDS: &[&str] = &[
    "init-db",
    "initdb",
    "init_db",
    "migrate",
    "makemigrations",
    "seed",
    "db:migrate",
    "db:seed",
    "db:setup",
    "db:reset",
    "createdb",
    "dropdb",
    "reset-db",
    "reset_db",
];

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SetupStep {
    InstallsPackages(String),
    ChangesData(String),
}

pub fn setup_steps_in(command: &str) -> Vec<SetupStep> {
    segments(command)
        .filter_map(|segment| {
            if installs::installs_packages(segment) {
                Some(SetupStep::InstallsPackages(segment.to_string()))
            } else if changes_data(segment) {
                Some(SetupStep::ChangesData(segment.to_string()))
            } else {
                None
            }
        })
        .collect()
}

pub fn explain(steps: &[SetupStep]) -> String {
    let listed: Vec<String> = steps
        .iter()
        .map(|step| match step {
            SetupStep::InstallsPackages(segment) => format!("`{segment}` installs packages"),
            SetupStep::ChangesData(segment) => format!("`{segment}` resets or migrates data"),
        })
        .collect();
    format!(
        "The preview was not changed: the start command must only start the app, because Harvest runs it again every time the preview starts or restarts. {}. Run those steps once with run_command, save lasting setup with save_setup_script, then configure the preview with only the command that starts the app.",
        listed.join("; ")
    )
}

fn segments(command: &str) -> impl Iterator<Item = &str> {
    command
        .split(['\n', ';'])
        .flat_map(|part| part.split("&&"))
        .flat_map(|part| part.split("||"))
        .map(str::trim)
        .filter(|segment| !segment.is_empty())
}

fn changes_data(segment: &str) -> bool {
    segment
        .split_whitespace()
        .map(|word| word.trim_matches(|c: char| !c.is_ascii_alphanumeric() && !matches!(c, ':' | '-' | '_')))
        .map(str::to_ascii_lowercase)
        .any(|word| DATA_CHANGING_WORDS.contains(&word.as_str()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_command_that_only_starts_the_app_is_accepted() {
        for command in [
            "npm run dev -- --host 0.0.0.0 --port $PORT",
            "cd examples/tutorial && flask --app flaskr run --host 0.0.0.0 --port 5000",
            "python3 -m http.server $PORT",
            "cargo run --release -- serve",
        ] {
            assert_eq!(setup_steps_in(command), [], "{command}");
        }
    }

    #[test]
    fn database_initialisation_before_the_start_is_found() {
        let command = "cd examples/tutorial && flask --app flaskr init-db && flask --app flaskr run --port 5000";
        assert_eq!(setup_steps_in(command), [SetupStep::ChangesData("flask --app flaskr init-db".into())]);
        assert_eq!(
            setup_steps_in("python manage.py migrate; python manage.py runserver"),
            [SetupStep::ChangesData("python manage.py migrate".into())]
        );
        assert_eq!(setup_steps_in("bin/rails db:seed && bin/rails server").len(), 1);
    }

    #[test]
    fn grouping_and_quotes_do_not_hide_a_data_step() {
        let command = "cd examples/tutorial && ([ -f instance/flaskr.sqlite ] || flask --app flaskr init-db) && flask run";
        assert_eq!(setup_steps_in(command), [SetupStep::ChangesData("flask --app flaskr init-db)".into())]);
        assert_eq!(setup_steps_in("sh -c 'python manage.py migrate'").len(), 1);
    }

    #[test]
    fn package_installs_before_the_start_are_found() {
        assert_eq!(
            setup_steps_in("npm ci && npm start"),
            [SetupStep::InstallsPackages("npm ci".into())]
        );
    }

    #[test]
    fn the_explanation_names_each_step_and_what_to_do_instead() {
        let text = explain(&[SetupStep::ChangesData("flask init-db".into())]);
        assert!(text.contains("`flask init-db` resets or migrates data"));
        assert!(text.contains("run_command"));
    }
}
