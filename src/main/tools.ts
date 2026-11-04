/**
 * Where the import pipeline finds its external tools.
 *
 * A repo-local virtualenv is preferred when it exists; otherwise the tool is
 * looked up on PATH. Virtualenvs lay out their executables differently per
 * platform, which is the one place the pipeline has to know about Windows.
 */
import fs from 'node:fs/promises';
import os from 'node:os';
import path from 'node:path';

export async function exists(file: string): Promise<boolean> {
  try {
    await fs.access(file);
    return true;
  } catch {
    return false;
  }
}

/** The path a virtualenv under `root` would use for the executable `name`. */
export function venvBin(root: string, name: string): string {
  return process.platform === 'win32'
    ? path.join(root, '.venv', 'Scripts', `${name}.exe`)
    : path.join(root, '.venv', 'bin', name);
}

/** The venv copy of `name` if the repo has one, otherwise `name` on PATH. */
export async function resolveTool(root: string, name: string): Promise<string> {
  const local = venvBin(root, name);
  return (await exists(local)) ? local : name;
}

/**
 * The Python interpreter plus any leading arguments: the venv interpreter
 * when present, otherwise `python` on Windows (python.org installs ship no
 * `python3`, and a bare `python3` is the Store stub) and `python3` elsewhere.
 */
export async function resolvePython(root: string): Promise<string[]> {
  const local = venvBin(root, 'python');
  if (await exists(local)) return [local];
  return process.platform === 'win32' ? ['python'] : ['python3'];
}

/** yt-dlp, plus the Deno runtime flag when a user-level Deno is installed. */
export async function ytDlpCommand(root: string): Promise<string[]> {
  const command = await resolveTool(root, 'yt-dlp');
  const deno = path.join(os.homedir(), '.deno', 'bin', process.platform === 'win32' ? 'deno.exe' : 'deno');
  return (await exists(deno)) ? [command, '--js-runtimes', `deno:${deno}`] : [command];
}
