#!/usr/bin/env bash

run_suite() {
	local suite="$1"

	local config_file="config/fuzzing${suite}.json"
	local mode="fuzzing${suite}"

	docker run \
		-it \
		--rm \
		-v "${PWD}/${config_file}:/${config_file}" \
		-v "${PWD}/reports:/reports" \
		--network host \
		--name ws-testsuite \
		crossbario/autobahn-testsuite wstest --mode "${mode}" -s "${config_file}"
}

main() {
	local test="$1"

	if [[ "$test" == "client" ]]; then
		run_suite client
	elif [[ "$test" == "server" ]]; then
		run_suite server
	else
		echo "available test modes: client, server"
	fi
}

main $@
