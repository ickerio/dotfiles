#!/bin/bash
#
# hermes/setup.sh — provision the locked-down `hermes` system user for Hermes Agent.
#
# Run as root:   sudo ./setup.sh
# Optional env:  WITH_DOCKER=1    also add hermes to the docker group + use the docker terminal backend
#                MAIN_USER=<name> add them to hshare and chmod 700 their home (never guessed)
#
# Arch Linux assumed (pacman for nodejs). Safe to re-run: every step checks
# state first and reports done vs skipped.
#
# What it automates: user/group/shared dir, minimal dotfiles, Claude Code CLI
# (native installer, per-user), the Hermes CLI itself, the
# claude-subscription-directsdk plugin, model provider selection, and the
# boot-time gateway service (user unit + linger).
# What stays manual (interactive): `claude auth login` (OAuth browser flow) and
# `hermes setup` (Telegram token etc.) — checked and printed at the end.

set -u

if [ "$EUID" -ne 0 ]; then
    echo "Please run as root"
    exit 1
fi

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
HERMES_INSTALL_URL="https://raw.githubusercontent.com/NousResearch/hermes-agent/main/scripts/install.sh"

done_msg() { echo "  [done]    $1"; }
skip_msg() { echo "  [skipped] $1"; }
warn_msg() { echo "  [warn]    $1" >&2; }

# run a command as the hermes user
as_hermes() { sudo -u hermes "$@"; }

echo "Provisioning hermes user"

# 1. shared group
if getent group hshare >/dev/null; then
    skip_msg "group hshare already exists"
else
    groupadd hshare
    done_msg "created group hshare"
fi

# 2. service user (no password -> no interactive login possible)
if id hermes >/dev/null 2>&1; then
    skip_msg "user hermes already exists"
else
    useradd -m -s /bin/bash hermes
    done_msg "created user hermes (no password, shell /bin/bash)"
fi

# 3. group memberships
usermod -aG hshare hermes
done_msg "ensured hermes is in group hshare"
if [ "${WITH_DOCKER:-0}" = "1" ]; then
    if getent group docker >/dev/null; then
        usermod -aG docker hermes
        done_msg "ensured hermes is in group docker (WITH_DOCKER=1)"
    else
        warn_msg "WITH_DOCKER=1 but no docker group on this system; skipping"
    fi
else
    skip_msg "docker group membership (set WITH_DOCKER=1 to add)"
fi

# 4. shared workspace directory
if [ -d /srv/hshare ]; then
    skip_msg "/srv/hshare already exists (re-applying ownership/perms below)"
fi
mkdir -p /srv/hshare
chown root:hshare /srv/hshare
chmod 2770 /srv/hshare
done_msg "ensured /srv/hshare (root:hshare, 2770 setgid)"
if command -v setfacl >/dev/null 2>&1; then
    setfacl -d -m g:hshare:rwx /srv/hshare
    done_msg "applied default ACL g:hshare:rwx on /srv/hshare"
else
    warn_msg "setfacl not found; default ACL skipped (install the 'acl' package for group-writable inheritance)"
fi

# 5. minimal dotfiles for the hermes user
cp "$SCRIPT_DIR/bashrc" /home/hermes/.bashrc
cp "$SCRIPT_DIR/bash_profile" /home/hermes/.bash_profile
cp "$SCRIPT_DIR/gitconfig" /home/hermes/.gitconfig
chown hermes:hermes /home/hermes/.bashrc /home/hermes/.bash_profile /home/hermes/.gitconfig
chmod 600 /home/hermes/.bashrc /home/hermes/.bash_profile /home/hermes/.gitconfig
chmod 700 /home/hermes
done_msg "installed .bashrc/.bash_profile/.gitconfig for hermes; locked /home/hermes to 700"

# 6. systemd-friendly env file (for EnvironmentFile= in the future unit)
mkdir -p /etc/hermes
printf '%s\n' 'HERMES_WRITE_SAFE_ROOT=/srv/hshare:/home/hermes/.hermes' > /etc/hermes/env
chown root:root /etc/hermes /etc/hermes/env
chmod 700 /etc/hermes
chmod 600 /etc/hermes/env
done_msg "wrote /etc/hermes/env (HERMES_WRITE_SAFE_ROOT, root-only 600)"

# 7. main user: group membership + home lockdown — only when explicitly requested
if [ -n "${MAIN_USER:-}" ]; then
    MAIN_HOME="$(getent passwd "$MAIN_USER" | cut -d: -f6)"
    usermod -aG hshare "$MAIN_USER"
    done_msg "ensured $MAIN_USER is in group hshare (log out/in for it to take effect)"
    if [ -n "$MAIN_HOME" ] && [ -d "$MAIN_HOME" ]; then
        chmod 700 "$MAIN_HOME"
        done_msg "locked $MAIN_HOME to 700 (MAIN_USER=$MAIN_USER)"
        if [ -e "$MAIN_HOME/hshare" ]; then
            skip_msg "~/hshare link (something already exists at $MAIN_HOME/hshare)"
        else
            sudo -u "$MAIN_USER" ln -s /srv/hshare "$MAIN_HOME/hshare"
            done_msg "linked ~/hshare -> /srv/hshare for $MAIN_USER"
        fi
    else
        warn_msg "MAIN_USER=$MAIN_USER has no resolvable home; skipping lockdown"
    fi
else
    skip_msg "main-user setup (set MAIN_USER=<name> to add them to hshare and chmod 700 their home)"
fi

# 8. Claude Code CLI via the native installer (per-user, as hermes).
# npm install is deprecated upstream; the native installer drops a
# self-contained binary at ~/.local/bin/claude (already on PATH via the
# managed bashrc) with background auto-updates. No Node.js needed.
if [ -x /home/hermes/.local/bin/claude ]; then
    skip_msg "claude CLI already installed (/home/hermes/.local/bin/claude)"
else
    if as_hermes bash -c 'curl -fsSL https://claude.ai/install.sh | bash'; then
        done_msg "installed Claude Code via native installer (as hermes)"
    else
        warn_msg "Claude Code native installer failed; see output above"
    fi
fi

# 9. Hermes CLI itself, installed as the hermes user
if as_hermes bash -lc 'command -v hermes' >/dev/null 2>&1; then
    skip_msg "hermes CLI already installed for user hermes"
else
    if curl -fsSL -o /tmp/hermes-install.sh "$HERMES_INSTALL_URL"; then
        chmod +x /tmp/hermes-install.sh
        if as_hermes bash /tmp/hermes-install.sh; then
            done_msg "installed hermes CLI as user hermes"
        else
            warn_msg "hermes installer exited non-zero; see output above"
        fi
        rm -f /tmp/hermes-install.sh
    else
        warn_msg "could not download the hermes installer"
    fi
fi

# resolve the hermes binary for the steps below (sudo -u keeps a minimal PATH,
# so prefer the login-shell resolution, then our managed ~/.local/bin)
HERMES_BIN=""
if HERMES_BIN="$(as_hermes bash -lc 'command -v hermes' 2>/dev/null)" && [ -n "$HERMES_BIN" ]; then
    skip_msg "hermes on PATH for hermes user ($HERMES_BIN)"
else
    HERMES_BIN=""
    for candidate in /home/hermes/.hermes/hermes-agent /home/hermes/.hermes/bin/hermes; do
        if [ -x "$candidate" ]; then
            as_hermes mkdir -p /home/hermes/.local/bin
            as_hermes ln -sf "$candidate" /home/hermes/.local/bin/hermes
            HERMES_BIN=/home/hermes/.local/bin/hermes
            done_msg "linked $candidate -> ~/.local/bin/hermes (on PATH via managed .bashrc)"
            break
        fi
    done
    if [ -z "$HERMES_BIN" ]; then
        warn_msg "hermes binary not found; steps 10-12 skipped (install it manually, then re-run)"
    fi
fi
hermes_u() { as_hermes "$HERMES_BIN" "$@"; }

# 10. claude-subscription-directsdk plugin + provider selection (as hermes)
if [ -n "$HERMES_BIN" ]; then
    if hermes_u plugins list 2>/dev/null | grep -q claude-subscription-directsdk; then
        skip_msg "claude-subscription-directsdk plugin already installed"
    else
        if hermes_u plugins install claude-subscription-directsdk; then
            done_msg "installed claude-subscription-directsdk plugin"
        else
            warn_msg "plugin install failed (requires hermes 0.21.4+); run manually as hermes: hermes plugins install claude-subscription-directsdk"
        fi
    fi
    # non-interactive equivalent of `hermes model`
    hermes_u config set model.provider claude-subscription-directsdk-experimental >/dev/null 2>&1
    hermes_u config set model.default sonnet >/dev/null 2>&1
    if as_hermes grep -q 'claude-subscription-directsdk-experimental' /home/hermes/.hermes/config.yaml 2>/dev/null; then
        done_msg "model provider -> claude-subscription-directsdk-experimental (default sonnet)"
    else
        warn_msg "could not confirm model provider in ~/.hermes/config.yaml; run 'hermes model' as the hermes user"
    fi
else
    skip_msg "plugin + provider setup (no hermes binary)"
fi

# 11. docker terminal backend (opt-in via WITH_DOCKER=1)
if [ "${WITH_DOCKER:-0}" = "1" ] && [ -n "$HERMES_BIN" ]; then
    hermes_u config set terminal.backend docker >/dev/null 2>&1
    if as_hermes grep -q 'backend: docker' /home/hermes/.hermes/config.yaml 2>/dev/null; then
        done_msg "terminal backend -> docker"
    else
        warn_msg "could not set docker terminal backend; run 'hermes setup terminal' as hermes"
    fi
    if hermes_u egress setup >/dev/null 2>&1; then
        done_msg "egress proxy set up (sandbox never sees real API keys)"
    else
        warn_msg "'hermes egress setup' needs attention; run it manually as hermes"
    fi
elif [ "${WITH_DOCKER:-0}" = "1" ]; then
    skip_msg "docker terminal backend (no hermes binary)"
else
    skip_msg "docker terminal backend (set WITH_DOCKER=1 to enable)"
fi

# 12. gateway service: user-scope unit + linger = auto-start at boot, no login needed
if [ -n "$HERMES_BIN" ]; then
    HUID="$(id -u hermes)"
    if hermes_u gateway install >/dev/null 2>&1; then
        done_msg "installed hermes-gateway user service (as hermes)"
    else
        warn_msg "'hermes gateway install' reported an issue; check manually as hermes"
    fi
    if loginctl enable-linger hermes 2>/dev/null; then
        done_msg "enabled linger for hermes (user services start at boot without login)"
    else
        warn_msg "loginctl enable-linger failed — without it the service stays down after a reboot with no login"
    fi
    # best effort: start it now (needs the user bus, which may not exist yet in this session)
    sleep 2
    if as_hermes env "XDG_RUNTIME_DIR=/run/user/$HUID" systemctl --user enable --now hermes-gateway >/dev/null 2>&1; then
        done_msg "hermes-gateway enabled and started"
    else
        warn_msg "could not start the service in this session (no user bus yet); linger will start it on next boot"
    fi
else
    skip_msg "gateway service install (no hermes binary)"
fi

# 13. auth status checks -> manual follow-ups below
echo ""
echo "=== Auth / config status ==="
if [ -x /home/hermes/.local/bin/claude ] && as_hermes /home/hermes/.local/bin/claude auth status >/dev/null 2>&1; then
    done_msg "claude CLI is logged in (Pro/Max subscription usable by hermes user)"
    NEED_CLAUDE_LOGIN=0
else
    warn_msg "claude CLI is NOT logged in for the hermes user"
    NEED_CLAUDE_LOGIN=1
fi
if [ -n "$HERMES_BIN" ] && as_hermes test -f /home/hermes/.hermes/config.yaml; then
    done_msg "hermes config exists (~/.hermes/config.yaml)"
else
    warn_msg "no hermes config yet — the setup wizard has not run"
fi

cat <<EOF

Manual follow-ups (interactive, cannot be scripted):
$([ "$NEED_CLAUDE_LOGIN" = "1" ] && echo "  1. Log the claude CLI into your Pro/Max subscription (OAuth browser flow):
       sudo -u hermes -i
       claude auth login" || echo "  1. claude login: already done")
  2. Run the Hermes setup wizard (Telegram bot token etc.):
       sudo -u hermes -i
       hermes setup
     (later: \`hermes setup --quick\` only prompts for missing items)
  3. Verify auto-start: reboot, then check with no login:
       sudo -u hermes -i
       systemctl --user status hermes-gateway
EOF
