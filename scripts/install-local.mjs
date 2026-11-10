#!/usr/bin/env node
// `corepack pnpm app:install`: builds the release app from this checkout and installs it for the
// current user, with Desktop and Start menu shortcuts. Windows only for now.
//
// The installed app is a normal release build: Scenario is the provider (OQ-PRV-20), the real
// prompt chat backend is on (PRM §13), app data lives in the default per-user folder and the
// library is the one you pick in the app. It shares nothing with dev runs, which use .pf-dev/.
//
//   --background   start the build detached and return at once (used by the git hook)
//   --hook         install a local post-merge hook in this clone: every merge or pull that
//                  lands on main rebuilds and reinstalls in the background. The hook lives in
//                  .git/hooks and is never committed.
//   --unhook       remove that hook
//
// Install folder: %LOCALAPPDATA%\Programs\Pocket Forge (override with PF_INSTALL_DIR).
// It holds pocket-forge.exe, version.txt (the commit it was built from) and install.log.
// A running app is never interrupted: the old exe is renamed aside and the new one is used
// from the next launch. A failed build leaves the installed app as it was.
import { spawn, spawnSync } from "node:child_process";
import {
  appendFileSync,
  copyFileSync,
  existsSync,
  mkdirSync,
  openSync,
  closeSync,
  readdirSync,
  readFileSync,
  renameSync,
  rmSync,
  statSync,
  unlinkSync,
  writeFileSync,
} from "node:fs";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const root = resolve(dirname(fileURLToPath(import.meta.url)), "..");
const args = new Set(process.argv.slice(2));

if (process.platform !== "win32") {
  console.error("app:install supports Windows only for now.");
  process.exit(1);
}
if (process.env.CI) {
  console.error("app:install refuses to run in CI.");
  process.exit(1);
}

const localAppData = process.env.LOCALAPPDATA;
if (!localAppData && !process.env.PF_INSTALL_DIR) {
  console.error("LOCALAPPDATA is not set; set PF_INSTALL_DIR.");
  process.exit(1);
}
const installDir =
  process.env.PF_INSTALL_DIR || join(localAppData, "Programs", "Pocket Forge");
const exeName = "pocket-forge.exe";
const logFile = join(installDir, "install.log");
const lockFile = join(installDir, "build.lock");
const againFile = join(installDir, "build.again");

const git = (...a) =>
  spawnSync("git", a, { cwd: root, encoding: "utf8" }).stdout?.trim() ?? "";

if (args.has("--hook") || args.has("--unhook")) {
  const hooksDir = resolve(root, git("rev-parse", "--git-path", "hooks"));
  const hook = join(hooksDir, "post-merge");
  const marker = "# pocket-forge app:install hook";
  if (args.has("--unhook")) {
    if (existsSync(hook) && readFileSync(hook, "utf8").includes(marker))
      unlinkSync(hook);
    console.log("Hook removed.");
    process.exit(0);
  }
  if (existsSync(hook) && !readFileSync(hook, "utf8").includes(marker)) {
    console.error(
      `A different post-merge hook exists at ${hook}; not replacing it.`,
    );
    process.exit(1);
  }
  mkdirSync(hooksDir, { recursive: true });
  writeFileSync(
    hook,
    [
      "#!/bin/sh",
      marker,
      "# Rebuilds and reinstalls the app in the background after a merge or pull on main.",
      '[ "$(git rev-parse --abbrev-ref HEAD)" = "main" ] || exit 0',
      "node scripts/install-local.mjs --background || true",
      "",
    ].join("\n"),
  );
  console.log(
    `Hook installed: merges and pulls on main now reinstall the app.`,
  );
  process.exit(0);
}

mkdirSync(installDir, { recursive: true });

if (args.has("--background")) {
  const child = spawn(process.execPath, [fileURLToPath(import.meta.url)], {
    cwd: root,
    detached: true,
    stdio: "ignore",
    windowsHide: true,
  });
  child.unref();
  process.exit(0);
}

function log(line) {
  const text = `${new Date().toISOString()} ${line}\n`;
  process.stdout.write(text);
  try {
    appendFileSync(logFile, text);
  } catch {
    // The log is best effort.
  }
}

// One build at a time. A request that arrives while a build runs asks it to go again, so the
// installed app always ends at the newest main. A lock older than an hour is from a dead build.
function takeLock() {
  try {
    if (
      existsSync(lockFile) &&
      Date.now() - statSync(lockFile).mtimeMs > 3_600_000
    )
      rmSync(lockFile, { force: true });
    closeSync(openSync(lockFile, "wx"));
    return true;
  } catch {
    writeFileSync(againFile, "");
    return false;
  }
}

if (!takeLock()) {
  log("A build is already running; it will build again when it finishes.");
  process.exit(0);
}

let failed = false;
try {
  do {
    rmSync(againFile, { force: true });
    if (!buildAndInstall()) failed = true;
  } while (existsSync(againFile));
} finally {
  rmSync(lockFile, { force: true });
}
process.exit(failed ? 1 : 0);

function buildAndInstall() {
  const commit = git("rev-parse", "--short", "HEAD");
  const dirty = git("status", "--porcelain") !== "";
  log(
    `Building ${commit}${dirty ? " (with uncommitted changes)" : ""} from ${root}`,
  );
  const env = { ...process.env };
  // Never let a dev or test setting leak into the installed build.
  for (const k of Object.keys(env)) if (k.startsWith("PF_")) delete env[k];
  const r = spawnSync(
    "corepack",
    ["pnpm", "exec", "tauri", "build", "--no-bundle"],
    {
      cwd: join(root, "app"),
      env,
      shell: true,
      encoding: "utf8",
      stdio: ["ignore", "pipe", "pipe"],
      maxBuffer: 256 * 1024 * 1024,
    },
  );
  if (r.status !== 0) {
    const tail = `${r.stdout ?? ""}${r.stderr ?? ""}`
      .split(/\r?\n/)
      .slice(-40)
      .join("\n");
    log(
      `Build FAILED (exit ${r.status}); the installed app is unchanged.\n${tail}`,
    );
    writeFileSync(
      join(installDir, "last-build-failed.txt"),
      `${commit}\n${tail}\n`,
    );
    return false;
  }
  rmSync(join(installDir, "last-build-failed.txt"), { force: true });

  const built = join(root, "target", "release", exeName);
  const target = join(installDir, exeName);
  // A running exe can be renamed but not overwritten: move it aside, then copy the new one.
  if (existsSync(target))
    renameSync(target, join(installDir, `pocket-forge.old-${Date.now()}.exe`));
  copyFileSync(built, target);
  writeFileSync(
    join(installDir, "version.txt"),
    `${commit}${dirty ? "+dirty" : ""}\n`,
  );
  for (const f of readdirSync(installDir)) {
    if (/^pocket-forge\.old-\d+\.exe$/.test(f)) {
      try {
        unlinkSync(join(installDir, f));
      } catch {
        // Still running; removed by a later install.
      }
    }
  }
  shortcuts(target);
  log(`Installed ${commit} to ${target}`);
  return true;
}

function shortcuts(target) {
  const ps = `
$ErrorActionPreference = 'Stop'
$shell = New-Object -ComObject WScript.Shell
$places = @([Environment]::GetFolderPath('Desktop'), (Join-Path ([Environment]::GetFolderPath('Programs')) ''))
foreach ($dir in $places) {
  $lnk = $shell.CreateShortcut((Join-Path $dir 'Pocket Forge.lnk'))
  $lnk.TargetPath = $env:PF_TARGET
  $lnk.WorkingDirectory = Split-Path $env:PF_TARGET
  $lnk.IconLocation = "$($env:PF_TARGET),0"
  $lnk.Description = 'Pocket Forge'
  $lnk.Save()
}`;
  const r = spawnSync(
    "powershell.exe",
    ["-NoProfile", "-NonInteractive", "-Command", ps],
    {
      env: { ...process.env, PF_TARGET: target },
      encoding: "utf8",
      windowsHide: true,
    },
  );
  if (r.status !== 0)
    log(`Shortcuts not created: ${(r.stderr || r.stdout).trim()}`);
}
