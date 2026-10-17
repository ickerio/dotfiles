#
# ~/.bashrc
#

# If not running interactively, don't do anything
[[ $- != *i* ]] && return

alias ls='ls --color=auto'
alias grep='grep --color=auto'

# Prompt: folder and git branch
. /usr/share/git/completion/git-prompt.sh
GIT_PS1_SHOWDIRTYSTATE=1
GIT_PS1_SHOWUNTRACKEDFILES=1

PS1='\[\e[1;34m\]\w\[\e[0m\]$(__git_ps1 " \[\e[35m\](%s)\[\e[0m\]") '

# Ctrl+R history search, Ctrl+T file picker, Alt+C cd into folder
command -v fzf >/dev/null && eval "$(fzf --bash)"
# `z <part of folder name>` jumps to frequently used folders
command -v zoxide >/dev/null && eval "$(zoxide init bash)"

# bun
export BUN_INSTALL="$HOME/.bun"
export PATH="$BUN_INSTALL/bin:$PATH"

alias aux="pactl set-default-sink alsa_output.pci-0000_00_1f.3.analog-stereo"
alias speakers="pactl set-default-sink alsa_output.usb-Audioengine_Ltd._Audioengine_2__ABCDEFB1180003-00.analog-stereo"

# Machine specific aliases, symlinked by install.sh (e.g. xps/bashrc.sh)
[[ -f ~/.bashrc.local ]] && . ~/.bashrc.local
