__enzui_status=$?
if [[ ${__enzui_shell_pid-} != "$$" ]]; then
    __enzui_shell_pid=$$
    __enzui_command_start() {
        builtin local LC_ALL=C line command hex='' i || builtin return 0
        line=$(HISTTIMEFORMAT= builtin history 1)
        [[ $line =~ ^[[:space:]]*([0-9]+)[[:space:]]+(.*)$ ]] || builtin return 0
        [[ ${BASH_REMATCH[1]} != "${__enzui_history_number-}" ]] || builtin return 0
        command=${BASH_REMATCH[2]}
        (( ${#command} > 0 && ${#command} <= 4096 )) || builtin return 0
        for ((i = 0; i < ${#command}; i++)); do
            builtin printf -v hex '%s%02x' "$hex" "'${command:i:1}"
        done
        builtin printf '\033]777;enzui-command;%s\007' "$hex"
    }
    __enzui_report_cwd() {
        builtin local LC_ALL=C path="$PWD" hex='' i || builtin return 0
        (( ${#path} > 0 && ${#path} <= 4096 )) || builtin return 0
        for ((i = 0; i < ${#path}; i++)); do
            builtin printf -v hex '%s%02x' "$hex" "'${path:i:1}"
        done
        builtin printf '\033]777;enzui-cwd;localhost;%s\007' "$hex"
    }
    # PS0 runs only after Bash accepts a command, never for a program's stdin.
    PS0='$(builtin declare -F __enzui_command_start >/dev/null && __enzui_command_start)'${PS0-}
fi
__enzui_history_line=$(HISTTIMEFORMAT= builtin history 1)
__enzui_history_number=''
if [[ $__enzui_history_line =~ ^[[:space:]]*([0-9]+)[[:space:]] ]]; then
    __enzui_history_number=${BASH_REMATCH[1]}
fi
builtin printf '\033]133;D;%d\007' "$__enzui_status"
builtin declare -F __enzui_report_cwd >/dev/null && __enzui_report_cwd
