const fs = require("node:fs");
const path = require("node:path");

const rootDir = path.resolve(__dirname, "..");
const runtimeDir = path.join(rootDir, "sync-runtime");
const runtimeScriptsDir = path.join(runtimeDir, "scripts");

function removePathSafely(targetPath) {
  if (!fs.existsSync(targetPath)) {
    return;
  }

  for (let attempt = 0; attempt < 5; attempt += 1) {
    try {
      fs.rmSync(targetPath, { recursive: true, force: true, maxRetries: 5, retryDelay: 200 });
      return;
    } catch (error) {
      if (attempt === 4) {
        break;
      }
    }
  }

  for (const entry of fs.readdirSync(targetPath, { withFileTypes: true })) {
    const childPath = path.join(targetPath, entry.name);
    if (entry.isDirectory()) {
      removePathSafely(childPath);
    } else {
      fs.rmSync(childPath, { force: true, maxRetries: 5, retryDelay: 200 });
    }
  }

  fs.rmdirSync(targetPath);
}

function resetRuntimeDir() {
  removePathSafely(runtimeDir);
  fs.mkdirSync(runtimeScriptsDir, { recursive: true });
}

function copyRequiredFiles() {
  fs.copyFileSync(
    path.join(rootDir, "scripts", "toutiao_sync.js"),
    path.join(runtimeScriptsDir, "toutiao_sync.js"),
  );

  fs.cpSync(
    path.join(rootDir, "node_modules"),
    path.join(runtimeDir, "node_modules"),
    { recursive: true },
  );
}

function main() {
  resetRuntimeDir();
  copyRequiredFiles();
  process.stdout.write("Prepared sync-runtime\n");
}

main();
