/**
 * Sync all saved data by re-running the tests for each version.
 *
 * This script should be used when the bench program or its tests has changed
 * and all data needs to be updated.
 */

import * as fs from "fs/promises";
import path from "path";

import {
  ANCHOR_VERSION_ARG,
  BenchData,
  LockFile,
  Toml,
  Version,
  VersionManager,
  getPlatformToolsVersion,
  spawn,
  usesLegacyIdl,
} from "./utils";

const CARGO_LOCK_PATH = "Cargo.lock";
const PROGRAM_MANIFEST_PATH = path.join("programs", "bench", "Cargo.toml");
(async () => {
  const bench = await BenchData.open();

  const cargoToml = await Toml.open(
    path.join("..", "programs", "bench", "Cargo.toml")
  );
  const anchorToml = await Toml.open(path.join("..", "Anchor.toml"));

  const unreleased = bench.get("unreleased");
  VersionManager.setSolanaVersion(unreleased.solanaVersion);

  const buildEnv = {
    ...process.env,
    RUSTC_BOOTSTRAP: "1",
    RUSTFLAGS: "-Z emit-stack-sizes",
  };

  for (const version of bench.getVersions()) {
    const platformToolsVersion = await getPlatformToolsVersion(
      bench.get(version).solanaVersion
    );
    bench.setPlatformToolsVersion(version, platformToolsVersion);
  }
  await bench.save();

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
    const buildResult = spawn("anchor", ["build", "--skip-lint"], {
      env: buildEnv,
    });
    if (buildResult.status !== 0) {
      throw new Error("Failed to build the current benchmark program.");
    }

    for (const version of bench.getVersions()) {
      console.log(`Updating '${version}'...`);

      await setProjectVersion(version);

      const cargoBuildSbfVersionResult = spawn(
        "cargo-build-sbf",
        ["--version"],
        {
          throwOnError: { msg: "Failed to read the platform-tools version." },
        }
      );
      const actualPlatformToolsVersion =
        /(?:sbf|platform)-tools (v\d+\.\d+)/.exec(
          cargoBuildSbfVersionResult.stdout.toString()
        )?.[1];
      const expectedPlatformToolsVersion =
        bench.get(version).platformToolsVersion;
      if (actualPlatformToolsVersion !== expectedPlatformToolsVersion) {
        throw new Error(
          `Expected platform-tools ${expectedPlatformToolsVersion}, found ${actualPlatformToolsVersion}.`
        );
      }

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

      const buildResult = spawn(
        "cargo-build-sbf",
        ["--manifest-path", PROGRAM_MANIFEST_PATH, "--", "--locked"],
        { env: buildEnv }
      );
      if (buildResult.status !== 0) {
        console.error("Please fix the error and re-run this command.");
        process.exitCode = 1;
        return;
      }

      const result = spawn(
        "anchor",
        ["test", "--skip-lint", "--skip-build", "--validator", "legacy"],
        { env: buildEnv }
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
  }
})();
