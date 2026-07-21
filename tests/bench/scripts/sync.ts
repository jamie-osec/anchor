/**
 * Sync all saved data by re-running the tests for each version.
 *
 * This script should be used when the bench program or its tests has changed
 * and all data needs to be updated.
 */

import * as fs from "fs/promises";
import os from "os";
import path from "path";

import {
  ANCHOR_VERSION_ARG,
  BenchData,
  LockFile,
  Toml,
  Version,
  VersionManager,
  spawn,
  usesLegacyIdl,
} from "./utils";

const CARGO_LOCK_PATH = "Cargo.lock";
const PROGRAM_MANIFEST_PATH = path.join("programs", "bench", "Cargo.toml");
const STACK_DEPENDENCY_UPDATES: Partial<
  Record<Version, [dependency: string, version: string][]>
> = {
  "0.27.0": [["ahash@0.7.6", "0.7.8"]],
  "0.28.0": [
    ["ahash@0.7.6", "0.7.8"],
    ["ahash@0.8.3", "0.8.7"],
  ],
  "0.29.0": [
    ["ahash@0.7.6", "0.7.8"],
    ["ahash@0.8.3", "0.8.7"],
  ],
};

(async () => {
  const bench = await BenchData.open();

  const cargoToml = await Toml.open(
    path.join("..", "programs", "bench", "Cargo.toml")
  );
  const anchorToml = await Toml.open(path.join("..", "Anchor.toml"));

  const unreleased = bench.get("unreleased");
  VersionManager.setSolanaVersion(unreleased.solanaVersion);

  const cargoBuildSbfResult = spawn("which", ["cargo-build-sbf"], {
    throwOnError: { msg: "Failed to find cargo-build-sbf." },
  });
  const cargoBuildSbfPath = await fs.realpath(
    cargoBuildSbfResult.stdout.toString().trim()
  );
  const tempBinPath = await fs.mkdtemp(path.join(os.tmpdir(), "anchor-bench-"));
  const tempCargoBuildSbfPath = path.join(tempBinPath, "cargo-build-sbf");
  await fs.copyFile(cargoBuildSbfPath, tempCargoBuildSbfPath);
  await fs.chmod(tempCargoBuildSbfPath, 0o755);
  spawn(
    "cp",
    [
      "-a",
      path.join(path.dirname(cargoBuildSbfPath), "platform-tools-sdk"),
      tempBinPath,
    ],
    { throwOnError: { msg: "Failed to copy the platform tools SDK." } }
  );
  const env = {
    ...process.env,
    PATH: `${tempBinPath}${path.delimiter}${process.env.PATH}`,
  };
  const setProjectVersion = async (version: Version) => {
    const isUnreleased = version === "unreleased";

    await LockFile.replace(version);
    VersionManager.setSolanaVersion(bench.get(version).solanaVersion);

    cargoToml.replaceValue("idl-build", () => {
      return usesLegacyIdl(version)
        ? "[]"
        : '["anchor-lang/idl-build", "anchor-spl/idl-build"]';
    });

    for (const dependency of ["lang", "spl"]) {
      cargoToml.replaceValue(`anchor-${dependency}`, () => {
        return isUnreleased
          ? `{ path = "../../../../${dependency}" }`
          : `"${version}"`;
      });
    }
    await cargoToml.save();

    anchorToml.replaceValue(
      "test",
      (cmd) => {
        return cmd.includes(ANCHOR_VERSION_ARG)
          ? cmd.replace(
              new RegExp(`\\s*${ANCHOR_VERSION_ARG}\\s+(.+)`),
              (arg, ver) => (isUnreleased ? "" : arg.replace(ver, version))
            )
          : isUnreleased
          ? cmd
          : `${cmd} ${ANCHOR_VERSION_ARG} ${version}`;
      },
      { insideQuotes: true }
    );
    await anchorToml.save();
  };

  try {
    // Older Anchor versions cannot generate IDLs using the current CLI. Create
    // one before switching dependencies so those versions can skip IDL builds.
    const buildResult = spawn("anchor", ["build", "--skip-lint"], { env });
    if (buildResult.status !== 0) {
      throw new Error("Failed to build the current benchmark program.");
    }

    for (const version of bench.getVersions()) {
      console.log(`Updating '${version}'...`);

      await setProjectVersion(version);

      // Resolve path dependencies in the cached lockfile before using the
      // version's Cargo. Keep the original lockfile format for old Cargo
      // versions that do not understand version 4.
      let lockFileVersion: string | undefined;
      try {
        const lockFile = await fs.readFile(CARGO_LOCK_PATH, "utf8");
        lockFileVersion = /^version = (\d+)$/m.exec(lockFile)?.[1];
        if (!lockFileVersion) {
          throw new Error("Failed to read lockfile version.");
        }
      } catch (err) {
        if (version !== "unreleased") throw err;
      }

      spawn(
        "cargo",
        [
          "metadata",
          "--format-version=1",
          "--features",
          "no-entrypoint",
          "--manifest-path",
          PROGRAM_MANIFEST_PATH,
        ],
        {
          maxBuffer: 16 * 1024 * 1024,
          throwOnError: { msg: "Failed to resolve benchmark dependencies." },
        }
      );
      if (lockFileVersion && lockFileVersion !== "4") {
        const resolvedLockFile = await fs.readFile(CARGO_LOCK_PATH, "utf8");
        await fs.writeFile(
          CARGO_LOCK_PATH,
          resolvedLockFile.replace(
            /^version = \d+$/m,
            `version = ${lockFileVersion}`
          )
        );
      }

      const buildResult = spawn("cargo-build-sbf", [
        "--manifest-path",
        PROGRAM_MANIFEST_PATH,
        "--",
        "--locked",
      ]);
      if (buildResult.status !== 0) {
        console.error("Please fix the error and re-run this command.");
        process.exitCode = 1;
        return;
      }

      // Rust 1.89 rejects dependencies used by the oldest releases. Update
      // only the live lockfile used for the stack metadata rebuild, leaving
      // the cached historical lockfiles unchanged.
      for (const [dependency, dependencyVersion] of STACK_DEPENDENCY_UPDATES[
        version
      ] ?? []) {
        spawn(
          "cargo",
          [
            "update",
            "--manifest-path",
            PROGRAM_MANIFEST_PATH,
            "-p",
            dependency,
            "--precise",
            dependencyVersion,
          ],
          {
            throwOnError: {
              msg: `Failed to update ${dependency} to ${dependencyVersion}.`,
            },
          }
        );
      }

      const result = spawn(
        "anchor",
        ["test", "--skip-lint", "--skip-build", "--validator", "legacy"],
        { env }
      );

      if (result.status !== 0) {
        console.error("Please fix the error and re-run this command.");
        process.exitCode = 1;
        return;
      }
    }

    spawn("anchor", ["run", "sync-markdown"]);
  } finally {
    await setProjectVersion("unreleased");
    await fs.rm(tempBinPath, { recursive: true });
  }
})();
