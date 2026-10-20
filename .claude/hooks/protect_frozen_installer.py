#!/usr/bin/env python3
"""Claude Code PreToolUse hook: keep the FROZEN Android installer untouched.

The Inhouse services updater's Android installer installed an update from
inside the app on a real Samsung phone on 2026-10-20, after weeks of broken
"improvements". Read FROZEN_INSTALLER.md. This hook blocks every file-tool
write to the frozen files and every shell command that writes, moves or
deletes them or touches tag installer-known-good or branch
frozen-installer. Exit code 2 blocks the tool
call and shows the reason to the agent. Only the user can lift the freeze, by
editing .claude/settings.json themselves.
"""
import json
import re
import sys

# Path suffixes of the frozen files, here and as vendored copies in the apps.
FROZEN = (
    'android/PhotosUpdateInstaller.kt',
    'android/UpdateFileProvider.java',
    'register_android.py',
    'FROZEN_INSTALLER.md',
    'test_frozen_installer.py',
    'workflows/frozen-installer.yml',
    '.claude/settings.json',
    '.claude/hooks/protect_frozen_installer.py',
)
ANCHORS = ('installer-known-good', 'frozen-installer')  # tag and branch of the verified commit
REASON = ('BLOCKED: {what} is part of the FROZEN Android installer, verified on a real phone. '
          'Do not change it, work around this hook or edit the guards. Read FROZEN_INSTALLER.md '
          'and, if the task really needs it, stop and ask the user.')

# Commands that change a file named anywhere in them.
DESTRUCTIVE = re.compile(
    r'\bsed\b[^|;&\n]*\s-[a-zA-Z]*i|\bperl\b[^|;&\n]*\s-[a-zA-Z]*i|\btee\b|\bmv\b|\brm\b|\bunlink\b|'
    r'\btruncate\b|\bdd\b|\bpatch\b|\bshred\b|\bchmod\b|\bln\b|'
    r'\bgit\s+(?:apply|am|checkout|restore|rm|mv|reset|stash|cherry-pick|revert|merge|rebase|update-index)\b|'
    r'\bwrite_text\b|\bwrite_bytes\b|\bopen\s*\([^)]*[\'"][wax+]|\.write\s*\(|\bos\.(?:remove|replace|rename|unlink)\b|'
    r'\bshutil\.|\.unlink\s*\(|\.rename\s*\(|\.replace\s*\(\s*[\'"]?/|\bfs\.(?:write|rm|unlink|rename|copy)')


def frozen_in(text):
    return [suffix for suffix in FROZEN if suffix in text]


def is_frozen_path(path):
    path = path.replace('\\', '/')
    return any(path == suffix or path.endswith('/' + suffix) for suffix in FROZEN)


def block(what):
    print(REASON.format(what=what), file=sys.stderr)
    sys.exit(2)


def main():
    try:
        event = json.load(sys.stdin)
    except ValueError:
        return
    tool, data = event.get('tool_name', ''), event.get('tool_input') or {}
    if tool in ('Edit', 'Write', 'MultiEdit', 'NotebookEdit'):
        path = data.get('file_path') or data.get('notebook_path') or ''
        if is_frozen_path(path):
            block(path)
    elif tool == 'Bash':
        command = data.get('command', '')
        for anchor in ANCHORS:
            if anchor in command and re.search(r'\bgit\s+(?:tag|push|update-ref|branch|replace)\b', command) \
                    and not re.search(r'\bgit\s+(?:tag|branch)\s+(?:-l|--list|-a|-r|-v)\b', command):
                block(anchor)
        named = frozen_in(command)
        if not named:
            return
        # Redirecting output into a frozen file, or a write-like command naming one.
        redirect = any(re.search(r'>>?\s*[\'"]?[^\s;&|]*' + re.escape(suffix), command) for suffix in named)
        copy_into = any(re.search(r'\b(?:cp|install|rsync)\b[^;&|\n]*\s[\'"]?[^\s;&|]*' + re.escape(suffix) + r'[\'"]?\s*(?:$|[;&|\n])', command)
                        for suffix in named)
        if redirect or copy_into or DESTRUCTIVE.search(command):
            block(', '.join(named))


if __name__ == '__main__':
    main()
