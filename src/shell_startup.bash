# The user's rc file runs first. Its exit status is not a verdict on the
# integration: a last line that tests something false is ordinary, and the
# checks below find whatever the integration actually needs.
if [[ -r $HOME/.bashrc ]]; then
    builtin source "$HOME/.bashrc"
fi
# Quoted declaration arguments keep assignment errors catchable instead of aborting the rc file.
if (( BASH_VERSINFO[0] > 4 || (BASH_VERSINFO[0] == 4 && BASH_VERSINFO[1] >= 4) )) &&
    [[ -o history ]] &&
    builtin history 0 >/dev/null &&
    builtin printf '' &&
    ( builtin declare -ga -- @ENZUI_PROMPT_ARRAY@ ) &&
    builtin shopt -s cmdhist lithist &&
    builtin unset -v __enzui_shell_pid __enzui_status __enzui_history_line __enzui_history_number &&
    builtin unset -f __enzui_command_start __enzui_report_cwd &&
    builtin declare -g -- "PS0=${PS0-}" &&
    builtin declare -ga -- @ENZUI_PROMPT_ARRAY@
then
    builtin :
else
    # Tell the terminal, silently: the shell works without the integration.
    builtin printf '\033]777;@ENZUI_HISTORY_DISABLED@\007' ||
        builtin echo -n $'\033]777;@ENZUI_HISTORY_DISABLED@\007'
fi
