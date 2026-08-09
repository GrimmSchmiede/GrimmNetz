use crate::ssh::SshSession;
use anyhow::{anyhow, Result};

/// Grants the SSH login user passwordless sudo, scoped to that one user via a dedicated
/// sudoers.d file. Required because our SSH commands run non-interactively (no TTY), so
/// `sudo` can never prompt for a password - without this, every privileged command
/// (creating the `gameserver` user, writing systemd units, starting services) silently
/// fails. Idempotent: safe to call on every connection, a no-op once already set up.
/// Uses the one password we already have to answer sudo's `-S` stdin prompt exactly once.
pub async fn ensure_passwordless_sudo(ssh: &mut SshSession, username: &str, password: &str) -> Result<()> {
    let check = ssh.exec("sudo -n true 2>&1; echo EXIT:$?").await?;
    if check.contains("EXIT:0") {
        return Ok(()); // already configured
    }

    // `sudo` must be the head of the exec'd process (no upstream pipe), otherwise whatever
    // feeds it becomes the thing consuming our piped stdin instead of sudo's password prompt.
    let script = format!(
        "sudo -S -p '' bash -c \"echo '{username} ALL=(ALL) NOPASSWD:ALL' > /etc/sudoers.d/grimmnetz && chmod 440 /etc/sudoers.d/grimmnetz\""
    );
    let stdin = format!("{password}\n");
    let output = ssh.exec_with_stdin(&script, stdin.as_bytes()).await?;

    let verify = ssh.exec("sudo -n true 2>&1; echo EXIT:$?").await?;
    if !verify.contains("EXIT:0") {
        return Err(anyhow::anyhow!(
            "Konnte passwortloses Sudo nicht einrichten (Passwort falsch oder Nutzer nicht sudo-berechtigt): {output}"
        ));
    }
    Ok(())
}

/// The Linux distro families we know how to provision. Add a new variant + match arm in
/// `detect_distro_family`/`bootstrap_server` to support another one - the rest of the app
/// (SSH, systemd units, game installs) is already distro-agnostic once this step succeeds.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DistroFamily {
    /// Ubuntu, Debian, and derivatives (apt/dpkg).
    Debian,
    /// Fedora, RHEL, CentOS Stream, Rocky, AlmaLinux (dnf).
    Fedora,
}

/// Reads `/etc/os-release`'s ID and ID_LIKE fields to classify the server into a distro
/// family. Checked against both fields since derivatives (e.g. Linux Mint, Rocky Linux)
/// often only declare their parent in ID_LIKE, not ID itself.
pub async fn detect_distro_family(ssh: &mut SshSession) -> Result<DistroFamily> {
    let raw = ssh
        .exec("grep -E '^(ID|ID_LIKE)=' /etc/os-release")
        .await?
        .to_lowercase();

    if raw.contains("debian") || raw.contains("ubuntu") {
        Ok(DistroFamily::Debian)
    } else if raw.contains("fedora") || raw.contains("rhel") || raw.contains("centos") {
        Ok(DistroFamily::Fedora)
    } else {
        Err(anyhow::anyhow!(
            "Diese Linux-Distribution wird noch nicht unterstützt (erkannt: {}). \
             Aktuell unterstützt: Ubuntu/Debian und Fedora/RHEL/CentOS/Rocky/AlmaLinux.",
            raw.trim()
        ))
    }
}

/// Creates the isolated `gameserver` system user (no root execution of game processes)
/// and installs base dependencies (Java, 32-bit libs for SteamCMD/game binaries) using
/// the right package manager for the detected distro family.
pub async fn bootstrap_server(ssh: &mut SshSession, timezone: Option<&str>) -> Result<()> {
    ssh.exec("id -u gameserver &>/dev/null || sudo useradd -m -s /bin/bash gameserver").await?;

    let family = detect_distro_family(ssh).await?;
    match family {
        DistroFamily::Debian => {
            ssh.exec("sudo apt-get update -y").await?;
            ssh.exec(
                "sudo apt-get install -y curl wget tar unzip jq openjdk-21-jre-headless openjdk-25-jre-headless \
                 software-properties-common lib32gcc-s1 lib32stdc++6 unattended-upgrades",
            )
            .await?;
            // Debian's unattended-upgrades package installs disabled by default - enabling it
            // needs a debconf answer, not just the package being present.
            ssh.exec(
                "echo 'unattended-upgrades unattended-upgrades/enable_auto_updates boolean true' | \
                 sudo debconf-set-selections && sudo dpkg-reconfigure -f noninteractive unattended-upgrades",
            )
            .await?;
        }
        DistroFamily::Fedora => {
            ssh.exec(
                "sudo dnf install -y curl wget tar unzip jq java-21-openjdk-headless java-latest-openjdk-headless \
                 glibc.i686 libstdc++.i686 ncurses-libs.i686 dnf-automatic",
            )
            .await?;
            ssh.exec(
                "sudo sed -i 's/^apply_updates = no/apply_updates = yes/' /etc/dnf/automatic.conf && \
                 sudo systemctl enable --now dnf-automatic.timer",
            )
            .await?;
        }
    }

    // SteamCMD installed via Valve's own tarball into a fixed path, identical across every
    // distro - sidesteps package-manager differences entirely (and the Ubuntu `steamcmd`
    // package's /usr/games/steamcmd not being on the non-interactive SSH $PATH, which bit
    // us before).
    ssh.exec(
        "test -x /home/gameserver/.steamcmd/steamcmd.sh || \
         sudo -u gameserver bash -c \"mkdir -p /home/gameserver/.steamcmd && \
         cd /home/gameserver/.steamcmd && \
         curl -sqL 'https://steamcdn-a.akamaihd.net/client/installer/steamcmd_linux.tar.gz' | tar zxf -\"",
    )
    .await?;

    ensure_swap(ssh).await?;
    limit_journal_size(ssh).await?;

    if let Some(tz) = timezone {
        // Only ever forward IANA-format identifiers (Region/City, letters/digits/_/+/-) -
        // never interpolate the raw string otherwise, this comes from the OS's own timezone
        // detection but there's no reason to trust it blindly in a shell command.
        if tz.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '/' | '_' | '+' | '-')) {
            let _ = ssh.exec(&format!("sudo timedatectl set-timezone {tz}")).await;
        }
    }

    Ok(())
}

/// Minecraft/Palworld/etc. can run for weeks and log constantly - without a cap, journald
/// logs can quietly fill the disk and take the whole VPS down. Idempotent (just (re)writes
/// the same config line).
async fn limit_journal_size(ssh: &mut SshSession) -> Result<()> {
    ssh.exec(
        "sudo mkdir -p /etc/systemd/journald.conf.d && \
         echo -e '[Journal]\\nSystemMaxUse=200M' | sudo tee /etc/systemd/journald.conf.d/grimmnetz.conf > /dev/null && \
         sudo systemctl restart systemd-journald",
    )
    .await?;
    Ok(())
}

/// Many cheap VPS images ship with zero swap configured - when RAM fills up (easy to hit
/// with a couple of gameserver instances), the kernel OOM-killer just kills the gameserver
/// process outright instead of the OS gracefully slowing down. Idempotent: does nothing if
/// swap already exists in any form (swap partition, existing swapfile, etc.).
pub async fn ensure_swap(ssh: &mut SshSession) -> Result<()> {
    let existing = ssh.exec("swapon --show --noheadings 2>/dev/null").await.unwrap_or_default();
    if !existing.trim().is_empty() {
        return Ok(()); // swap already configured, nothing to do
    }

    ssh.exec(
        "sudo fallocate -l 2G /swapfile || sudo dd if=/dev/zero of=/swapfile bs=1M count=2048 && \
         sudo chmod 600 /swapfile && \
         sudo mkswap /swapfile && \
         sudo swapon /swapfile && \
         grep -q '/swapfile' /etc/fstab || echo '/swapfile none swap sw 0 0' | sudo tee -a /etc/fstab > /dev/null",
    )
    .await?;

    Ok(())
}

/// Generates and installs a systemd unit so the game server survives reboots
/// and is controlled purely via `systemctl` (start/stop/restart), running as `gameserver`.
pub fn render_systemd_unit(
    instance_id: &str,
    working_dir: &str,
    start_command: &str,
    ram_limit_mb: u32,
    cpu_limit_percent: u32,
) -> String {
    // systemd's ExecStart requires an absolute path or a $PATH-resolvable name - it does NOT
    // resolve "./binary" against WorkingDirectory like a shell would. Wrapping in `bash -c`
    // lets every game template keep writing simple relative start commands.
    let escaped_command = start_command.replace('\'', "'\\''");
    // Enforced by the kernel via cgroups, not just stored as UI-only metadata - MemoryMax hard-
    // kills the process on overrun (OOM), MemoryHigh throttles well before that point so a
    // spike doesn't instantly take the whole instance down. CPUQuota=200% means 2 full cores.
    let memory_high_mb = (ram_limit_mb as f64 * 0.9) as u32;
    format!(
        "[Unit]\n\
         Description=GrimmNetz Gameserver Instance {instance_id}\n\
         After=network.target\n\n\
         [Service]\n\
         Type=simple\n\
         User=gameserver\n\
         WorkingDirectory={working_dir}\n\
         ExecStart=/bin/bash -c '{escaped_command}'\n\
         StandardInput=fifo:/run/grimmnetz-{instance_id}.console\n\
         MemoryMax={ram_limit_mb}M\n\
         MemoryHigh={memory_high_mb}M\n\
         CPUQuota={cpu_limit_percent}%\n\
         Restart=on-failure\n\
         RestartSec=5\n\
         KillSignal=SIGTERM\n\
         TimeoutStopSec=60\n\
         SuccessExitStatus=143\n\n\
         [Install]\n\
         WantedBy=multi-user.target\n"
    )
}

pub async fn install_systemd_unit(ssh: &mut SshSession, unit_name: &str, unit_contents: &str) -> Result<()> {
    let escaped = unit_contents.replace('\'', "'\\''");
    ssh.exec(&format!(
        "echo '{escaped}' | sudo tee /etc/systemd/system/{unit_name}.service > /dev/null"
    ))
    .await?;
    ssh.exec("sudo systemctl daemon-reload").await?;
    ssh.exec(&format!("sudo systemctl enable {unit_name}")).await?;
    Ok(())
}

/// Checks whether a supported firewall is currently active - used right after adding a
/// server to decide whether to prompt "your server is unprotected, activate a firewall?".
pub async fn is_firewall_active(ssh: &mut SshSession, family: DistroFamily) -> Result<bool> {
    match family {
        DistroFamily::Debian => {
            let status = ssh.exec("sudo ufw status 2>/dev/null").await.unwrap_or_default();
            Ok(status.contains("Status: active"))
        }
        DistroFamily::Fedora => {
            let status = ssh
                .exec("systemctl is-active firewalld 2>/dev/null")
                .await
                .unwrap_or_default();
            Ok(status.trim() == "active")
        }
    }
}

/// Turns on the firewall for a server that doesn't have one active, with a safety net: before
/// touching anything, schedules an `at` job that reverts the firewall in 2 minutes, so a
/// mistake here (or an unrelated network hiccup) self-heals into "not locked out" instead of
/// "locked out forever". The SSH port is always allowed *before* the firewall is switched to
/// default-deny. Returns the scheduled `at` job id so the caller can cancel it once a fresh
/// connection confirms the server is still reachable.
pub async fn enable_firewall_with_rollback(ssh: &mut SshSession, family: DistroFamily, ssh_port: u16) -> Result<String> {
    let rollback_cmd = match family {
        DistroFamily::Debian => "sudo ufw --force disable",
        DistroFamily::Fedora => "sudo systemctl stop firewalld",
    };

    match family {
        DistroFamily::Debian => {
            ssh.exec("command -v ufw &>/dev/null || sudo apt-get install -y ufw").await?;
        }
        DistroFamily::Fedora => {}
    }
    ssh.exec("command -v at &>/dev/null || (sudo apt-get install -y at || sudo dnf install -y at)")
        .await?;
    ssh.exec("sudo systemctl enable --now atd").await?;

    let schedule_output = ssh
        .exec(&format!("echo '{rollback_cmd}' | sudo at now + 2 minutes 2>&1"))
        .await?;
    let job_id = schedule_output
        .split_whitespace()
        .skip_while(|w| *w != "job")
        .nth(1)
        .ok_or_else(|| anyhow!("Konnte geplanten Rollback-Job nicht auslesen: {schedule_output}"))?
        .to_string();

    match family {
        DistroFamily::Debian => {
            ssh.exec(&format!("sudo ufw allow {ssh_port}/tcp")).await?;
            // Don't trust the allow command silently succeeding - actually check the rule
            // landed before ever switching the firewall to default-deny. If `ufw allow`
            // swallowed an error, enabling now would lock the SSH port out completely.
            // Must use `ufw show added`, not `ufw status` - the latter only ever lists rules
            // while the firewall is already active, so checking it here (before `ufw enable`)
            // would always come back empty and falsely abort every single time.
            let added = ssh.exec("sudo ufw show added 2>/dev/null").await.unwrap_or_default();
            if !added.lines().any(|l| l.contains(&format!("allow {ssh_port}/tcp"))) {
                let _ = cancel_firewall_rollback(ssh, &job_id).await;
                return Err(anyhow!(
                    "SSH-Port {ssh_port}/tcp wurde nicht in den ufw-Regeln gefunden - \
                     Firewall wurde zur Sicherheit NICHT aktiviert."
                ));
            }
            ssh.exec("sudo ufw --force enable").await?;
        }
        DistroFamily::Fedora => {
            ssh.exec(&format!("sudo firewall-cmd --permanent --add-port={ssh_port}/tcp"))
                .await?;
            // Same verification for firewalld: confirm the permanent rule actually exists
            // before the service (default-deny once running) gets started.
            let ports = ssh
                .exec("sudo firewall-cmd --permanent --list-ports 2>/dev/null")
                .await
                .unwrap_or_default();
            if !ports.split_whitespace().any(|p| p == format!("{ssh_port}/tcp")) {
                let _ = cancel_firewall_rollback(ssh, &job_id).await;
                return Err(anyhow!(
                    "SSH-Port {ssh_port}/tcp wurde nicht in den firewalld-Regeln gefunden - \
                     Firewall wurde zur Sicherheit NICHT aktiviert."
                ));
            }
            ssh.exec("sudo systemctl enable --now firewalld").await?;
            ssh.exec("sudo firewall-cmd --reload").await?;
        }
    }

    Ok(job_id)
}

/// Cancels a scheduled firewall-rollback job - call once a fresh connection has confirmed the
/// server is still reachable after `enable_firewall_with_rollback`.
pub async fn cancel_firewall_rollback(ssh: &mut SshSession, job_id: &str) -> Result<()> {
    ssh.exec(&format!("sudo atrm {job_id} 2>/dev/null")).await?;
    Ok(())
}

/// Opens a port in whichever firewall manager is actually active on the server, so a freshly
/// installed (or reconfigured) game is reachable without the user ever touching a firewall
/// themselves. Best-effort by design: if no supported firewall is active (very common on
/// Debian/Ubuntu cloud images - no local firewall at all, filtering happens at the provider's
/// network edge instead), this is a silent no-op rather than a failure, since there's nothing
/// wrong to report - the port simply isn't blocked locally in the first place.
pub async fn open_port(ssh: &mut SshSession, family: DistroFamily, port: u16, protocol: &str) -> Result<()> {
    match family {
        DistroFamily::Debian => {
            // `ufw` ships inactive-by-default on most Debian/Ubuntu images; only touch it if
            // the admin already turned it on, otherwise leave it alone entirely.
            let status = ssh.exec("sudo ufw status 2>/dev/null").await.unwrap_or_default();
            if status.contains("Status: active") {
                ssh.exec(&format!("sudo ufw allow {port}/{protocol}")).await?;
            }
        }
        DistroFamily::Fedora => {
            // firewalld is active by default on Fedora/RHEL-family systems.
            let status = ssh
                .exec("systemctl is-active firewalld 2>/dev/null")
                .await
                .unwrap_or_default();
            if status.trim() == "active" {
                ssh.exec(&format!(
                    "sudo firewall-cmd --permanent --add-port={port}/{protocol} && sudo firewall-cmd --reload"
                ))
                .await?;
            }
        }
    }
    Ok(())
}

pub async fn control_instance(ssh: &mut SshSession, unit_name: &str, action: &str) -> Result<String> {
    // action: "start" | "stop" | "restart"
    ssh.exec(&format!("sudo systemctl {action} {unit_name}")).await
}
