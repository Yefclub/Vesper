//! Which copy of Vesper this process is, told apart by its bundle identifier.
//!
//! The QA build is how an agent runs a branch before a pull request exists, on
//! the same machine as the installed app. It must not open that app's meetings,
//! read its API key or take its global shortcut. Every one of those decisions is
//! made here, from the identifier `tauri.qa.conf.json` builds with — a second
//! switch for any of them is a switch somebody forgets to flip.

/// The identifier `tauri.qa.conf.json` gives the QA build.
pub const QA_IDENTIFIER: &str = "com.yefclub.vesper.qa";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Profile {
    /// The app people install, from the stable channel or the dev one.
    Standard,
    /// A branch under test, running beside the installed app.
    Qa,
}

impl Profile {
    /// Exact match only. Anything else keeps the behaviour every build before
    /// this one had.
    pub fn from_identifier(identifier: &str) -> Self {
        if identifier == QA_IDENTIFIER {
            Profile::Qa
        } else {
            Profile::Standard
        }
    }

    /// The folder under the OS data directory holding the database, recordings,
    /// models, backends and logs. Stable and dev have always shared one, and
    /// still do.
    pub fn data_dir_name(self) -> &'static str {
        match self {
            Profile::Standard => "Vesper",
            Profile::Qa => "Vesper QA",
        }
    }

    /// The keychain service the API key is stored under. The standard one is
    /// what every installed copy already wrote to; renaming it would lose each
    /// user's stored key on upgrade.
    pub fn keychain_service(self) -> &'static str {
        match self {
            Profile::Standard => "com.yefclub.vesper",
            Profile::Qa => QA_IDENTIFIER,
        }
    }

    /// Whether this copy may hold a system-wide key combination. A QA build
    /// holding the record shortcut would start recording in the test copy when
    /// the user meant the installed one.
    pub fn takes_global_shortcut(self) -> bool {
        self == Profile::Standard
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn identifier_in(config: &str) -> String {
        let config: serde_json::Value = serde_json::from_str(config).expect("config parses");
        config["identifier"]
            .as_str()
            .expect("config names an identifier")
            .to_string()
    }

    #[test]
    fn the_qa_config_builds_the_qa_profile() {
        let identifier = identifier_in(include_str!("../../tauri.qa.conf.json"));
        assert_eq!(Profile::from_identifier(&identifier), Profile::Qa);
    }

    #[test]
    fn stable_and_dev_channel_builds_are_standard() {
        for config in [
            include_str!("../../tauri.conf.json"),
            include_str!("../../tauri.dev.conf.json"),
        ] {
            let identifier = identifier_in(config);
            assert_eq!(Profile::from_identifier(&identifier), Profile::Standard);
        }
    }

    #[test]
    fn only_the_exact_identifier_is_qa() {
        for identifier in [
            "com.yefclub.vesper.qa.x",
            "com.yefclub.vesper.QA",
            " com.yefclub.vesper.qa",
            "",
        ] {
            assert_eq!(
                Profile::from_identifier(identifier),
                Profile::Standard,
                "{identifier:?}"
            );
        }
    }

    #[test]
    fn the_standard_key_stays_where_installed_copies_stored_it() {
        let identifier = identifier_in(include_str!("../../tauri.conf.json"));
        assert_eq!(Profile::Standard.keychain_service(), identifier);
    }

    #[test]
    fn qa_shares_no_data_key_or_shortcut_with_the_installed_app() {
        assert_ne!(
            Profile::Qa.data_dir_name(),
            Profile::Standard.data_dir_name()
        );
        assert_ne!(
            Profile::Qa.keychain_service(),
            Profile::Standard.keychain_service()
        );
        assert!(Profile::Standard.takes_global_shortcut());
        assert!(!Profile::Qa.takes_global_shortcut());
    }
}
