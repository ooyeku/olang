#!/bin/bash
# Sourced by tools/release.sh and tools/release-check.sh.
#
# semver_gt A B: A is newer than B (2.10.0 > 2.9.0, 1.0.0 > 1.0.0-rc.9,
# rc.10 > rc.9), as install.sh orders releases.
semver_gt() {
    local a=${1#v} b=${2#v}
    [ "$a" = "$b" ] && return 1
    local ac=${a%%-*} bc=${b%%-*} ap="" bp=""
    case "$a" in *-*) ap=${a#*-} ;; esac
    case "$b" in *-*) bp=${b#*-} ;; esac
    local i x y
    for i in 1 2 3; do
        x=$(echo "$ac" | cut -d. -f$i) y=$(echo "$bc" | cut -d. -f$i)
        [ "$x" -gt "$y" ] && return 0
        [ "$x" -lt "$y" ] && return 1
    done
    [ -z "$ap" ] && [ -n "$bp" ] && return 0
    [ -n "$ap" ] && [ -z "$bp" ] && return 1
    # both prereleases of one version: compare dot by dot, numbers as numbers
    local IFS=.
    local -a pa=($ap) pb=($bp)
    for ((i = 0; i < ${#pa[@]} || i < ${#pb[@]}; i++)); do
        x=${pa[i]:-} y=${pb[i]:-}
        [ -z "$x" ] && return 1
        [ -z "$y" ] && return 0
        if [[ $x =~ ^[0-9]+$ && $y =~ ^[0-9]+$ ]]; then
            [ "$x" -gt "$y" ] && return 0
            [ "$x" -lt "$y" ] && return 1
        else
            [[ $x > $y ]] && return 0
            [[ $x < $y ]] && return 1
        fi
    done
    return 1
}
