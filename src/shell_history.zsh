[[ -o interactive ]] || builtin return 1
__enzui_preexec() {
    builtin emulate -L zsh || builtin return 0
    builtin local LC_ALL=C command=$1 hex='' i || builtin return 0
    (( ${#command} > 0 && ${#command} <= 4096 )) || builtin return 0
    for ((i = 1; i <= ${#command}; i++)); do
        builtin printf -v hex '%s%02x' "$hex" "'${command[i]}"
    done
    builtin printf '\033]777;enzui-command;%s\007' "$hex"
}
__enzui_report_cwd() {
    builtin emulate -L zsh || builtin return 0
    builtin local LC_ALL=C directory=$PWD hex='' i || builtin return 0
    (( ${#directory} > 0 && ${#directory} <= 4096 )) || builtin return 0
    for ((i = 1; i <= ${#directory}; i++)); do
        builtin printf -v hex '%s%02x' "$hex" "'${directory[i]}"
    done
    builtin printf '\033]777;enzui-cwd;localhost;%s\007' "$hex"
}
__enzui_precmd() {
    builtin local result=$? || builtin return 0
    builtin printf '\033]133;D;%d\007' "$result"
    builtin typeset -f __enzui_report_cwd >/dev/null && __enzui_report_cwd
}
preexec_functions=(${preexec_functions:#__enzui_preexec} __enzui_preexec)
precmd_functions=(__enzui_precmd ${precmd_functions:#__enzui_precmd})
