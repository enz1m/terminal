builtin status is-interactive; or builtin return
# Predefine Fish's private reporters so lazy terminal discovery cannot register duplicate cwd events.
function __fish_update_cwd_osc
end
function __fish_report_cwd
end
function __enzui_hex
    builtin test (builtin string length -- "$argv[1]") -le 4096; or builtin return 1
    builtin set -l escaped (builtin string escape --style=url -- "$argv[1]")
    builtin test (builtin string length -- "$escaped") -le 12288; or builtin return 1
    builtin set -l bytes (builtin string match --all --regex '%[0-9A-Fa-f]{2}|.' -- "$escaped")
    builtin test (builtin count $bytes) -le 4096; or builtin return 1
    for byte in $bytes
        if builtin string match --quiet '%*' -- "$byte"
            builtin printf '%s' (builtin string sub --start 2 -- "$byte")
        else
            builtin printf '%02x' "'$byte"
        end
    end
end
function __enzui_preexec --on-event fish_preexec
    builtin functions --query __enzui_hex; or builtin return 1
    builtin set -l hex (__enzui_hex "$argv[1]" | builtin string collect)
    builtin string match -qr '^[0-9a-fA-F]+$' -- "$hex"; or builtin return
    builtin test (builtin string length -- "$hex") -le 8192; or builtin return
    builtin printf '\033]777;enzui-command;%s\007' "$hex"
end
function __enzui_report_cwd
    builtin functions --query __enzui_hex; or builtin return 1
    builtin set -l hex (__enzui_hex "$PWD" | builtin string collect)
    builtin string match -qr '^[0-9a-fA-F]+$' -- "$hex"; or builtin return
    builtin test (builtin string length -- "$hex") -le 8192; or builtin return
    builtin printf '\033]777;enzui-cwd;localhost;%s\007' "$hex"
end
function __enzui_prompt --on-event fish_prompt
    builtin set -l result $status
    builtin printf '\033]133;D;%d\007' "$result"
    builtin functions --query __enzui_report_cwd; and __enzui_report_cwd
end
builtin set -e ENZUI_SHELL_STARTUP
builtin functions --query __enzui_hex __enzui_preexec __enzui_report_cwd __enzui_prompt; or builtin return 1
builtin test (__enzui_hex '/' | builtin string collect) = 2f; or builtin return 1
builtin return 0
