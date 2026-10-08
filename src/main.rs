mod atomic;
mod paths;
mod profile;
mod state;

use anyhow::{bail, Context, Result};
use clap::{Parser, Subcommand};

use crate::profile::FileStatus;

#[derive(Parser)]
#[command(
    name = "codex-switch",
    version,
    about = "Switch Codex model/provider/API-key profiles without touching sandbox, approval, or trusted-project settings"
)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// List all profiles, marking the active one.
    #[command(alias = "ls")]
    List,
    /// Show the currently active profile.
    Current,
    /// Apply a profile's provider/model/auth layer over the live config.
    ///
    /// Only `model`, `model_provider`, the profile's `[model_providers.*]`
    /// entry, `model_catalog_json`, and `auth.json` are changed. Sandbox mode,
    /// approval policy, trusted projects, and other live settings are kept.
    Use {
        /// Profile name to activate.
        name: String,
    },
    /// Save the current live provider/model/auth settings into a profile.
    ///
    /// With no name, writes into the active profile. Captures the managed
    /// fields only; sandbox/approval/trusted-project settings are not stored.
    Save {
        /// Profile to overwrite. Defaults to the active profile.
        name: Option<String>,
    },
    /// Import the current live provider/model/auth settings as a new profile.
    Import {
        /// New profile name.
        name: String,
        /// Overwrite if the profile already exists.
        #[arg(long)]
        force: bool,
        /// Activate the new profile after importing.
        #[arg(long)]
        activate: bool,
    },
    /// Show how a profile differs from the live config.
    Diff {
        /// Profile to compare. Defaults to the active profile.
        name: Option<String>,
    },
    /// Remove a profile.
    #[command(alias = "remove")]
    Rm {
        /// Profile name to delete.
        name: String,
        /// Remove even if it is the active profile.
        #[arg(long)]
        force: bool,
    },
    /// Print resolved paths (live files and store location).
    Paths,
}

fn main() {
    if let Err(e) = run() {
        eprintln!("error: {e:#}");
        std::process::exit(1);
    }
}

fn run() -> Result<()> {
    let cli = Cli::parse();
    match cli.command {
        Command::List => cmd_list(),
        Command::Current => cmd_current(),
        Command::Use { name } => cmd_use(&name),
        Command::Save { name } => cmd_save(name.as_deref()),
        Command::Import {
            name,
            force,
            activate,
        } => cmd_import(&name, force, activate),
        Command::Diff { name } => cmd_diff(name.as_deref()),
        Command::Rm { name, force } => cmd_rm(&name, force),
        Command::Paths => cmd_paths(),
    }
}

fn cmd_list() -> Result<()> {
    let profiles = profile::list_profiles()?;
    let active = state::load_state()?.active;
    if profiles.is_empty() {
        println!("no profiles yet — create one with `codex-switch import <name>`");
        return Ok(());
    }
    for name in profiles {
        let marker = if active.as_deref() == Some(&name) {
            "*"
        } else {
            " "
        };
        println!("{marker} {name}");
    }
    Ok(())
}

fn cmd_current() -> Result<()> {
    match state::load_state()?.active {
        Some(name) => println!("{name}"),
        None => println!("(none)"),
    }
    Ok(())
}

fn cmd_use(name: &str) -> Result<()> {
    state::validate_profile_name(name)?;
    if !profile::profile_exists(name)? {
        bail!("profile not found: {name}");
    }
    let layer = profile::read_profile(name)?;
    if layer.is_empty() {
        bail!("profile {name:?} has no auth.json, config.provider.toml, or model-catalog.json");
    }

    // Capture live before touching it so a later step can roll back.
    let previous_live = profile::capture_live()?;

    profile::write_live(name, &layer)?;

    // Switching live and recording the active profile must agree. If we can't
    // persist the new active profile, restore live so disk and state stay
    // consistent.
    if let Err(state_err) = state::set_active(Some(name)) {
        if let Err(restore_err) = profile::write_live(name, &previous_live) {
            return Err(state_err.context(format!(
                "failed to record active profile, and rolling back live config also failed \
                 (live now = {name:?}, state unchanged): {restore_err:#}"
            )));
        }
        return Err(state_err.context("failed to record active profile; rolled back live config"));
    }
    println!("switched to {name}");
    println!("restart Codex to reload the profile: `codex app-server daemon restart`");
    Ok(())
}

fn cmd_save(name: Option<&str>) -> Result<()> {
    let target = match name {
        Some(n) => n.to_string(),
        None => state::load_state()?
            .active
            .context("no active profile; specify a name: `codex-switch save <name>`")?,
    };
    state::validate_profile_name(&target)?;

    let layer = profile::capture_live()?;
    if layer.is_empty() {
        bail!("no live Codex config found to save");
    }
    profile::write_profile(&target, &layer)?;
    println!("saved live provider settings into profile {target}");
    Ok(())
}

fn cmd_import(name: &str, force: bool, activate: bool) -> Result<()> {
    state::validate_profile_name(name)?;
    if profile::profile_exists(name)? && !force {
        bail!("profile {name:?} already exists; pass --force to overwrite");
    }
    let layer = profile::capture_live()?;
    if layer.is_empty() {
        bail!("no live Codex config found to import");
    }
    profile::write_profile(name, &layer)?;
    println!("imported live provider settings as profile {name}");
    if activate {
        state::set_active(Some(name))?;
        println!("set {name} as active");
    }
    Ok(())
}

fn cmd_diff(name: Option<&str>) -> Result<()> {
    let target = match name {
        Some(n) => n.to_string(),
        None => state::load_state()?
            .active
            .context("no active profile; specify a name: `codex-switch diff <name>`")?,
    };
    state::validate_profile_name(&target)?;
    if !profile::profile_exists(&target)? {
        bail!("profile not found: {target}");
    }

    let layer = profile::read_profile(&target)?;
    let result = profile::diff_profile(&layer)?;
    println!("auth.json:            {}", status_label(&result.auth));
    println!("config.provider.toml: {}", status_label(&result.fragment));
    println!("model-catalog.json:   {}", status_label(&result.catalog));
    if matches!(result.auth, FileStatus::Same)
        && matches!(result.fragment, FileStatus::Same)
        && matches!(result.catalog, FileStatus::Same)
    {
        println!("profile {target} matches live");
    }
    Ok(())
}

fn cmd_rm(name: &str, force: bool) -> Result<()> {
    state::validate_profile_name(name)?;
    profile::remove_profile(name, force)?;
    println!("removed profile {name}");
    Ok(())
}

fn cmd_paths() -> Result<()> {
    let (store, source) = paths::store_root_with_source()?;
    let source = match source {
        paths::StoreSource::Env => "CODEX_SWITCH_HOME",
        paths::StoreSource::Existing => "existing store",
        paths::StoreSource::Default => "default (~/.codex-switch)",
        paths::StoreSource::XdgFallback => "XDG fallback (home not writable)",
    };
    println!("codex home:   {}", paths::codex_home()?.display());
    println!("live auth:    {}", paths::codex_auth_path()?.display());
    println!("live config:  {}", paths::codex_config_path()?.display());
    println!("store root:   {}", store.display());
    println!("store source: {source}");
    println!("profiles dir: {}", paths::profiles_dir()?.display());
    Ok(())
}

fn status_label(status: &FileStatus) -> String {
    match status {
        FileStatus::Same => "same".to_string(),
        FileStatus::Differs => "differs".to_string(),
        FileStatus::OnlyProfile => "only in profile (missing from live)".to_string(),
        FileStatus::OnlyLive => "only in live (missing from profile)".to_string(),
        FileStatus::Neither => "absent in both".to_string(),
    }
}
