# Print an optspec for argparse to handle cmd's options that are independent of any subcommand.
function __fish_pcx_global_optspecs
	string join \n h/help V/version
end

function __fish_pcx_needs_command
	# Figure out if the current invocation already has a command.
	set -l cmd (commandline -opc)
	set -e cmd[1]
	argparse -s (__fish_pcx_global_optspecs) -- $cmd 2>/dev/null
	or return
	if set -q argv[1]
		# Also print the command, so this can be used to figure out what it is.
		echo $argv[1]
		return 1
	end
	return 0
end

function __fish_pcx_using_subcommand
	set -l cmd (__fish_pcx_needs_command)
	test -z "$cmd"
	and return 1
	contains -- $cmd[1] $argv
end

complete -c pcx -n "__fish_pcx_needs_command" -s h -l help -d 'Print help (see more with \'--help\')'
complete -c pcx -n "__fish_pcx_needs_command" -s V -l version -d 'Print version'
complete -c pcx -n "__fish_pcx_needs_command" -f -a "info" -d 'Show MCAP container metadata without decoding point frames'
complete -c pcx -n "__fish_pcx_needs_command" -f -a "topics" -d 'Discover Topics, MCAP Channels, Schemas, and message counts'
complete -c pcx -n "__fish_pcx_needs_command" -f -a "extract" -d 'Extract exactly one ROS 2 PointCloud2 Point Frame as PCD'
complete -c pcx -n "__fish_pcx_needs_command" -f -a "passthrough" -d 'Copy one selected encoded message into a faithful reduced MCAP'
complete -c pcx -n "__fish_pcx_needs_command" -f -a "help" -d 'Print this message or the help of the given subcommand(s)'
complete -c pcx -n "__fish_pcx_using_subcommand info" -l json -d 'Print versioned JSON instead of human-readable text'
complete -c pcx -n "__fish_pcx_using_subcommand info" -s h -l help -d 'Print help'
complete -c pcx -n "__fish_pcx_using_subcommand topics" -l json -d 'Print versioned JSON instead of human-readable text'
complete -c pcx -n "__fish_pcx_using_subcommand topics" -s h -l help -d 'Print help'
complete -c pcx -n "__fish_pcx_using_subcommand extract" -l topic -d 'Topic whose messages are counted as Point Frames' -r
complete -c pcx -n "__fish_pcx_using_subcommand extract" -l frame -d 'Zero-based Point Frame index after Topic selection' -r
complete -c pcx -n "__fish_pcx_using_subcommand extract" -l at -d 'First Point Frame at or after this duration from recording start' -r
complete -c pcx -n "__fish_pcx_using_subcommand extract" -s o -l output -d 'Output PCD path, or \'-\' for binary-safe stdout' -r -F
complete -c pcx -n "__fish_pcx_using_subcommand extract" -l encoding -d 'PCD payload representation' -r -f -a "binary\t''
ascii\t''"
complete -c pcx -n "__fish_pcx_using_subcommand extract" -l memory-limit -d 'Hard managed-memory limit in bytes' -r
complete -c pcx -n "__fish_pcx_using_subcommand extract" -l force -d 'Replace an existing output file'
complete -c pcx -n "__fish_pcx_using_subcommand extract" -s h -l help -d 'Print help'
complete -c pcx -n "__fish_pcx_using_subcommand passthrough" -l topic -d 'Topic whose encoded messages are selected' -r
complete -c pcx -n "__fish_pcx_using_subcommand passthrough" -l frame -d 'Zero-based message index after Topic selection' -r
complete -c pcx -n "__fish_pcx_using_subcommand passthrough" -l at -d 'First message at or after this duration from recording start' -r
complete -c pcx -n "__fish_pcx_using_subcommand passthrough" -s o -l output -d 'Output MCAP path, or \'-\' for binary-safe stdout' -r -F
complete -c pcx -n "__fish_pcx_using_subcommand passthrough" -l compression -d 'Deterministic output chunk compression' -r -f -a "none\t''
zstd\t''
lz4\t''"
complete -c pcx -n "__fish_pcx_using_subcommand passthrough" -l memory-limit -d 'Hard managed-memory limit in bytes' -r
complete -c pcx -n "__fish_pcx_using_subcommand passthrough" -l force -d 'Replace an existing output file'
complete -c pcx -n "__fish_pcx_using_subcommand passthrough" -s h -l help -d 'Print help'
complete -c pcx -n "__fish_pcx_using_subcommand help; and not __fish_seen_subcommand_from info topics extract passthrough help" -f -a "info" -d 'Show MCAP container metadata without decoding point frames'
complete -c pcx -n "__fish_pcx_using_subcommand help; and not __fish_seen_subcommand_from info topics extract passthrough help" -f -a "topics" -d 'Discover Topics, MCAP Channels, Schemas, and message counts'
complete -c pcx -n "__fish_pcx_using_subcommand help; and not __fish_seen_subcommand_from info topics extract passthrough help" -f -a "extract" -d 'Extract exactly one ROS 2 PointCloud2 Point Frame as PCD'
complete -c pcx -n "__fish_pcx_using_subcommand help; and not __fish_seen_subcommand_from info topics extract passthrough help" -f -a "passthrough" -d 'Copy one selected encoded message into a faithful reduced MCAP'
complete -c pcx -n "__fish_pcx_using_subcommand help; and not __fish_seen_subcommand_from info topics extract passthrough help" -f -a "help" -d 'Print this message or the help of the given subcommand(s)'
