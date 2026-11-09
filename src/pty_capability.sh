# portable-pty closes inherited descriptors; reopen the parent's retained descriptor, not the FIFO name.
cap=$1
shift
# A compound-command redirection failure reaches the fallback instead of terminating a POSIX shell.
if {
    if [ -p /proc/self/fd/9 ] && [ /proc/self/fd/9 -ef "$cap" ] && printf . >&9; then
        exec "$@"
    fi
    exec 9>&-
    exec "$@"
} 9<>"$cap"; then
    exit 0
fi
exec "$@"
