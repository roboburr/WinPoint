#!/usr/bin/python3 -I
"""GW1 tool stub (design 4.11): log argv, then fail.

tests/update/run.sh copies this file into ``<run>/guard-bin/`` (first on PATH) and ``<run>/tools/``
(WIN11_UPDATE_TOOL_DIR) and makes one symlink per tool name to it, so ``basename(argv[0])`` is the tool. The
log is ``<run>/stub-calls.jsonl``, found from the stub's own location (no environment variable, so a caller
that clears the environment is still logged). One JSON object per call:
``{"t", "tool", "dir", "argv", "cwd", "env", "rc"}``.

Behaviour:
* guard-bin: always exit 97 with a message on stderr.
* tools: if ``<tools>/.responses/<tool>.json`` exists (written by the fake's Load() from the timeline's
  ``stubs`` map), print its ``stdout``/``stderr`` and exit with its ``rc``; otherwise exit 97. The file is one
  response ``{"rc", "stdout", "stderr"}`` for every argv, or a list of them, each with ``"argv": [...]`` (equal)
  or ``"argvPrefix": [...]`` (the first arguments): the first entry that matches answers, and an argv no entry
  matches exits 97 with a message (D-G73: apt-cache answers ``policy <name:arch>...``, ``policy ?phasing`` and
  ``show`` differently).
* tools ``nice`` and ``ionice`` are pass-throughs: they skip their own options and exec the named program,
  but only one that exists in the same tools directory (never PATH, never an absolute path).
* stdin is never read or recorded: every program the update service starts gets its input as argv (D-G73), and
  a stub that read stdin could wait on, or take, input that belongs to its caller.
"""
import json
import os
import sys
import time

BLOCKED = 97
ENV_KEYS = ('LANGUAGE', 'LANG', 'LC_ALL', 'LC_MESSAGES', 'WIN11_PRESS_ENTER', 'PKEXEC_UID', 'DBUS_SYSTEM_BUS_ADDRESS')


def main() -> int:
    path = os.path.abspath(sys.argv[0])
    here = os.path.dirname(path)
    tool = os.path.basename(path)
    kind = os.path.basename(here)
    run = os.path.dirname(here)
    argv = sys.argv[1:]
    rc, out, err, exec_target = BLOCKED, '', f'win11 update stub: {tool} is blocked in the test layer ({kind})\n', None
    if kind == 'tools' and tool in ('nice', 'ionice'):
        target, rest = passthrough(tool, argv)
        if target and os.path.basename(target) == target and os.path.isfile(os.path.join(here, target)) \
                and target not in ('nice', 'ionice', 'stub.py'):
            exec_target = (os.path.join(here, target), rest)
            rc, err = 0, ''
        else:
            err = f'win11 update stub: {tool} target {target!r} is not a stub in {here}\n'
    elif kind == 'tools':
        response = os.path.join(here, '.responses', tool + '.json')
        try:
            with open(response, encoding='utf-8') as f:
                spec = answer(json.load(f), argv)
            if spec is None:
                err = f'win11 update stub: {tool} has no response for this argv in {response}\n'
            else:
                rc, out, err = int(spec.get('rc', 0)), spec.get('stdout', ''), spec.get('stderr', '')
        except FileNotFoundError:
            pass
        except (OSError, ValueError, TypeError, AttributeError) as e:
            err = f'win11 update stub: bad response file {response}: {e}\n'
    record = {'t': round(time.time(), 3), 'tool': tool, 'dir': kind, 'argv': argv, 'cwd': os.getcwd(),
              'env': {k: os.environ[k] for k in ENV_KEYS if k in os.environ}, 'rc': rc}
    if exec_target:
        record['exec'] = os.path.basename(exec_target[0])
    line = json.dumps(record, ensure_ascii=False) + '\n'
    try:
        fd = os.open(os.path.join(run, 'stub-calls.jsonl'), os.O_WRONLY | os.O_APPEND | os.O_CREAT, 0o600)
        try:
            os.write(fd, line.encode('utf-8'))
        finally:
            os.close(fd)
    except OSError as e:
        sys.stderr.write(f'win11 update stub: cannot write the stub log: {e}\n')
        return BLOCKED
    if exec_target:
        target, rest = exec_target
        os.execv(target, [target] + rest)
    if out:
        sys.stdout.write(out)
    if err:
        sys.stderr.write(err)
    return rc


def answer(spec, argv: list):
    """The response for argv: the one object, or the first list entry whose argv or argvPrefix matches; None
    when no entry matches."""
    if isinstance(spec, dict):
        return spec
    for entry in spec:
        prefix = entry.get('argvPrefix', [])
        if entry.get('argv', argv) == argv and argv[:len(prefix)] == prefix:
            return entry
    return None


def passthrough(tool: str, argv: list) -> tuple:
    """(program, its args) after nice's / ionice's own options."""
    args = list(argv)
    with_value = {'nice': ('-n', '--adjustment'), 'ionice': ('-c', '--class', '-n', '--classdata', '-p', '--pid',
                                                             '-P', '--pgid', '-u', '--uid')}[tool]
    while args and args[0].startswith('-'):
        opt = args.pop(0)
        if opt == '--':
            break
        if opt in with_value and args:
            args.pop(0)
    return (args[0], args[1:]) if args else (None, [])


if __name__ == '__main__':
    sys.exit(main())
