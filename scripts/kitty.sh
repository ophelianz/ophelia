#!/usr/bin/env bash
set -euo pipefail

SCRIPT_PATH="scripts/kitty.sh"
DEFAULT_MESSAGE="bugs behave plz"
DEFAULT_TAB_SIZE="4"

usage() {
    cat <<'EOF'
Usage:
  scripts/kitty.sh [--message "cute message"] [--tab-size N] [file-or-directory ...]

Behavior:
  - With file paths, prepends the Ophelia header to each file.
  - With directory paths, recursively updates every `.rs` file inside them.
  - If a file already starts with one or more Ophelia headers, they are collapsed into one.
  - With no file paths, prints the generated header to stdout.
EOF
}

message="$DEFAULT_MESSAGE"
tab_size="$DEFAULT_TAB_SIZE"
files=()

while (($# > 0)); do
    case "$1" in
        --message)
            shift
            if (($# == 0)); then
                echo "missing value for --message" >&2
                exit 1
            fi
            message="$1"
            ;;
        --tab-size)
            shift
            if (($# == 0)); then
                echo "missing value for --tab-size" >&2
                exit 1
            fi
            tab_size="$1"
            ;;
        -h|--help)
            usage
            exit 0
            ;;
        *)
            files+=("$1")
            ;;
    esac
    shift
done

generate_header() {
    local header_message="$1"

    printf '/***************************************************\n'
    printf '** This file is part of Ophelia, distributed under the\n'
    printf '** terms of the GPL License, version 3 or later.\n'
    printf '**\n'
    kittysay --think --tab-size "$tab_size" "$header_message" | sed '/^$/d' | sed 's/^/** /'
    printf '**************************************************/\n'
}

apply_header() {
    local file="$1"
    local header_file temp_file
    header_file="$(mktemp)"
    temp_file="$(mktemp)"

    generate_header "$message" > "$header_file"

    awk -v header_path="$header_file" '
        function flush_block() {
            if (block_is_ophelia) {
                skipped_any = 1
            } else {
                body = body block
                at_top = 0
            }
            block = ""
            block_is_ophelia = 0
            in_block = 0
        }

        BEGIN {
            while ((getline line < header_path) > 0) {
                header = header line ORS
            }
            close(header_path)
            at_top = 1
            in_block = 0
            skipped_any = 0
            body = ""
            block = ""
            block_is_ophelia = 0
        }

        {
            if (at_top) {
                if (in_block) {
                    block = block $0 ORS
                    if ($0 ~ /This file is part of Ophelia, distributed under the/) {
                        saw_ophelia = 1
                    }
                    if ($0 ~ /terms of the GPL License, version 3 or later\./) {
                        saw_gpl = 1
                    }
                    if ($0 ~ /\*\/[[:space:]]*$/) {
                        block_is_ophelia = saw_ophelia && saw_gpl
                        flush_block()
                        saw_ophelia = 0
                        saw_gpl = 0
                    }
                    next
                }

                if ($0 ~ /^[[:space:]]*$/ && skipped_any) {
                    next
                }

                if ($0 ~ /^\/\*/) {
                    in_block = 1
                    block = $0 ORS
                    saw_ophelia = ($0 ~ /This file is part of Ophelia, distributed under the/)
                    saw_gpl = ($0 ~ /terms of the GPL License, version 3 or later\./)
                    if ($0 ~ /\*\/[[:space:]]*$/) {
                        block_is_ophelia = saw_ophelia && saw_gpl
                        flush_block()
                        saw_ophelia = 0
                        saw_gpl = 0
                    }
                    next
                }

                at_top = 0
            }

            body = body $0 ORS
        }

        END {
            printf "%s", header
            if (length(body) > 0) {
                printf "\n%s", body
            }
        }
    ' "$file" > "$temp_file"

    mv "$temp_file" "$file"
    rm -f "$header_file"
}

collect_rust_files() {
    local path="$1"

    if [[ -f "$path" ]]; then
        printf '%s\n' "$path"
        return
    fi

    if [[ -d "$path" ]]; then
        find "$path" -type f -name '*.rs' | sort
        return
    fi

    echo "skipping missing path: $path" >&2
}

if ((${#files[@]} == 0)); then
    generate_header "$message"
    exit 0
fi

for path in "${files[@]}"; do
    while IFS= read -r file; do
        [[ -n "$file" ]] || continue
        apply_header "$file"
    done < <(collect_rust_files "$path")
done
