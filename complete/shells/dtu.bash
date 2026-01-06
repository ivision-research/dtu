__dtu_complete() {

    local nwords=${#COMP_WORDS[@]}
    local drop=$((nwords - COMP_CWORD))

    completions=$(env DTUC_SHELL=bash DTUC_DROP="$drop" DTUC_INCOMPLETE="${COMP_WORDS[$COMP_CWORD]}" dtu-complete "${COMP_WORDS[@]}")
    local IFS=$'\n'
    for completion in $completions; do
        COMPREPLY+=($completion)
    done
}

complete -F __dtu_complete dtu
