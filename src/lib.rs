pub mod guard;
pub mod patterns;
pub mod survey;

pub use guard::{CheckResult, InstallResult, UninstallResult, check_staged, install_hook, uninstall_hook};
pub use survey::{RepoChaff, Strain, survey, survey_repo};
