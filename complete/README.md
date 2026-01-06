# dtu-complete

This provides an easy way to get detailed completions from your shell. For example, `dtu call system-service -s <TAB>` will complete the `<SERVICE>` from the database if available. To incorporate this with your own shell:

- Set `DTUC_SHELL` to a flavor that is similar to your own. If nothing is similar, you'll need to update this program and make a pull request
- Set `DTUC_INCOMPLETE` to the word that is currently being completed
- Optionally set `DTUC_SKIP` to the amount of args that should be skipped (defaults to 2 to skip `dtu-complete` and `dtu`) and `DTUC_DROP` to the arguments that should be dropped off the end (defaults to 0)
- Call `dtu-complete ALL ARGS BEFORE DTUC_INCOMPLETE` -- do _not_ include `DTUC_INCOMPLETE` or anything after it with the args! `DTUC_DROP` should be used if your shell bundles all args together (like [the included Bash script](./shells/dtu.bash) does)
