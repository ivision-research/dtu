function __dtu_completions
    set -gx DTUC_SHELL fish
    set -gx DTUC_INCOMPLETE (commandline -ct)
    set -l args (commandline -opc)
    dtu-complete $args
end

complete -f -c dtu -a '(__dtu_completions)'
