#!/bin/sh
# check-config.sh <fragment> <.config>: fails, naming each one, when an option
# the fragment decides didn't end up that way in the final kernel config.
#
# A fragment line is only a request: olddefconfig drops a "not set" that
# another enabled option selects or that a hidden default forces back (the
# media TV/radio options stay on unless MEDIA_SUPPORT_FILTER is set, and
# DM_RAID selects BLK_DEV_MD), and a value whose dependencies are off. Both
# happened silently before this check. Run after configuring
# (external.mk's RAZE_CHECK_KERNEL_CONFIG).
set -u

fragment=$1
config=$2
[ -r "$fragment" ] && [ -r "$config" ] || {
	echo "check-config.sh: usage: check-config.sh <fragment> <.config>" >&2
	exit 2
}

# One line per option that didn't take: "CONFIG_X: wanted A, got B".
bad=$(awk '
	FNR == NR {
		if ($0 ~ /^# CONFIG_[A-Za-z0-9_]+ is not set$/)
			want[$2] = "n"
		else if ($0 ~ /^CONFIG_[A-Za-z0-9_]+=/)
			want[substr($0, 1, index($0, "=") - 1)] = substr($0, index($0, "=") + 1)
		next
	}
	/^CONFIG_[A-Za-z0-9_]+=/ { have[substr($0, 1, index($0, "=") - 1)] = substr($0, index($0, "=") + 1) }
	END {
		for (name in want) {
			got = (name in have) ? have[name] : "n"
			if (want[name] == "n" ? (got == "y" || got == "m") : (got != want[name]))
				printf "%s: wanted %s, got %s\n", name, want[name], got
		}
	}
' "$fragment" "$config" | sort)

[ -z "$bad" ] && exit 0
echo "Raze: kernel options in raze.config that did not take effect (another enabled option selects or forces them; see its Kconfig):" >&2
printf '%s\n' "$bad" | sed 's/^/  /' >&2
exit 1
