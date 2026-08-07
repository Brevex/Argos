#!/usr/bin/env bash
# PreToolUse hook (A-READ-ONLY guardrail): denies Bash commands that could write to a
# block device. Loopback *files* and /dev/null|zero|urandom stay allowed.
set -euo pipefail

cmd=$(jq -r '.tool_input.command // empty')
[[ -z "$cmd" ]] && exit 0

blockdev='/dev/(sd[a-z]|hd[a-z]|nvme[0-9]|mmcblk[0-9]|vd[a-z]|loop[0-9]|r?disk[0-9])'
destructive='(^|[;&|[:space:]("`])(dd|dcfldd|mkfs[.[:alnum:]]*|shred|wipefs|blkdiscard|sgdisk|sfdisk|fdisk|parted|hdparm|badblocks)([[:space:]]|$)'

if [[ "$cmd" =~ $blockdev ]]; then
  if [[ "$cmd" =~ $destructive ]] || [[ "$cmd" =~ (\>|of=)[[:space:]]*/dev/ ]]; then
    jq -n '{hookSpecificOutput:{hookEventName:"PreToolUse",permissionDecision:"deny",permissionDecisionReason:"A-READ-ONLY: commands that write to block devices are forbidden in this repo. Use file-backed fixtures instead."}}'
    exit 0
  fi
fi
exit 0
