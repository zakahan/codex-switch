mod atomic;
mod paths;
mod profile;
mod state;

use anyhow::{bail, Context, Result};
use clap::{Parser, Subcommand};

#[derive(Parser)]
#[command(
    name = "codex-switch",
    version,
    about = "Snapshot and switch Codex auth.json + config.toml profiles"
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
    /// Switch the live Codex config to a profile (backs up current live first).
    Use {
        /// Profile name to activate.
        name: String,
    },
    /// Save the current live config back into a profile.
    ///
    /// With no name, writes into the active profile. Captures drift.
    Save {
        /// Profile to overwrite. Defaults to the active profile.
        name: Option<String>,
    },
    /// Import the current live config as a new profile.
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
    /// Show which files differ between a profile and the live config.
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
    let snap = profile::read_profile(name)?;
    if snap.is_empty() {
        bail!("profile {name:?} has no auth.json or config.toml");
    }

    // Capture live before touching it so a later step can roll back.
    let previous_live = profile::read_live()?;

    if let Some(dir) = profile::backup_live()? {
        eprintln!("backed up current live config to {}", dir.display());
    }

    profile::write_live(&snap)?;

    // Switching live and recording the active profile must agree. If we can't
    // persist the new active profile, restore live so disk and state stay
    // consistent rather than leaving `current` pointing at the old profile
    // while the files are the new one.
    if let Err(state_err) = state::set_active(Some(name)) {
        if let Err(restore_err) = profile::write_live(&previous_live) {
            return Err(state_err.context(format!(
                "failed to record active profile, and rolling back live config also failed \
                 (live now = {name:?}, state unchanged): {restore_err:#}"
            )));
        }
        return Err(state_err.context("failed to record active profile; rolled back live config"));
    }
    println!("switched to {name}");
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

    let live = profile::read_live()?;
    if live.is_empty() {
        bail!("no live Codex config found to save");
    }
    profile::write_profile(&target, &live)?;
    println!("saved live config into profile {target}");
    Ok(())
}

fn cmd_import(name: &str, force: bool, activate: bool) -> Result<()> {
    state::validate_profile_name(name)?;
    if profile::profile_exists(name)? && !force {
        bail!("profile {name:?} already exists; pass --force to overwrite");
    }
    let live = profile::read_live()?;
    if live.is_empty() {
        bail!("no live Codex config found to import");
    }
    profile::write_profile(name, &live)?;
    println!("imported live config as profile {name}");
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

    let prof = profile::read_profile(&target)?;
    let live = profile::read_live()?;

    let auth = file_status(prof.auth.as_deref(), live.auth.as_deref());
    let config = file_status(prof.config.as_deref(), live.config.as_deref());
    println!("auth.json:   {auth}");
    println!("config.toml: {config}");
    if matches!(auth, FileStatus::Same) && matches!(config, FileStatus::Same) {
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

enum FileStatus {
    Same,
    Differs,
    OnlyProfile,
    OnlyLive,
    Neither,
}

impl std::fmt::Display for FileStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let s = match self {
            FileStatus::Same => "same",
            FileStatus::Differs => "differs",
            FileStatus::OnlyProfile => "only in profile (missing from live)",
            FileStatus::OnlyLive => "only in live (missing from profile)",
            FileStatus::Neither => "absent in both",
        };
        f.write_str(s)
    }
}

fn file_status(profile: Option<&[u8]>, live: Option<&[u8]>) -> FileStatus {
    match (profile, live) {
        (Some(a), Some(b)) if a == b => FileStatus::Same,
        (Some(_), Some(_)) => FileStatus::Differs,
        (Some(_), None) => FileStatus::OnlyProfile,
        (None, Some(_)) => FileStatus::OnlyLive,
        (None, None) => FileStatus::Neither,
    }
}
