pub mod gitignore;
pub mod guard;
pub mod patterns;
pub mod repair;
pub mod survey;

pub use gitignore::{GitignorePlan, RepoType, SynthesisResult, detect_repo_type, plan,
    render_gitignore, synthesize_gitignore};
pub use guard::{CheckResult, InstallResult, UninstallResult, check_staged, install_hook, uninstall_hook};
pub use repair::{RepairVerdict, repair, repair_all};
pub use survey::{RepoChaff, Strain, survey, survey_repo};
