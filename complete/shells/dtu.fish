function __dtu_completions
    set -l incomplete (commandline -ct)
    set -l args (commandline -opc)
    env DTUC_SHELL=fish DTUC_INCOMPLETE="$incomplete" dtu-complete $args
end

complete -f -c dtu -a '(__dtu_completions)'
