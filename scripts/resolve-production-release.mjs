#!/usr/bin/env node

import { appendFileSync } from "node:fs";
import { execFileSync } from "node:child_process";
import { fileURLToPath } from "node:url";
import { verifyReleaseVersions } from "./release-version.mjs";

const defaultRoot = fileURLToPath(new URL("../", import.meta.url));

function command(
  executable,
  args,
  { cwd = defaultRoot, env = process.env, quiet = false } = {},
) {
  return execFileSync(executable, args, {
    cwd,
    env,
    encoding: "utf8",
    stdio: ["ignore", "pipe", quiet ? "pipe" : "inherit"],
  }).trim();
}

function writeOutputs(values, output = process.env.GITHUB_OUTPUT) {
  if (!output) {
    console.log(JSON.stringify(values));
    return;
  }
  appendFileSync(
    output,
    `${Object.entries(values)
      .map(([name, value]) => `${name}=${value}`)
      .join("\n")}\n`,
  );
}

export function completePublishedAssetSet(assets, version) {
  const names = new Set(assets.map((asset) => asset.name));
  if (assets.length !== 5 || names.size !== 5) return false;
  const dmgs = [...names].filter((name) =>
    new RegExp(`^Bridge_${version.replace(/\./g, "\\.")}_(aarch64|x64)\\.dmg$`).test(name),
  );
  if (dmgs.length !== 1) return false;
  const stem = dmgs[0].slice(0, -".dmg".length);
  return [
    `${stem}.dmg`,
    `${stem}.dmg.sha256`,
    `${stem}.app.tar.gz`,
    `${stem}.app.tar.gz.sig`,
    "latest.json",
  ].every((name) => names.has(name));
}

export function releasePullRequestTitle(config, version) {
  const packageConfig = config.packages?.["."] || {};
  const configured = (name, fallback) => packageConfig[name] ?? config[name] ?? fallback;
  const component = configured("include-component-in-tag", true)
    ? packageConfig.component || packageConfig["package-name"] || ""
    : "";
  const componentText = component
    ? configured("component-no-space", false)
      ? component
      : ` ${component}`
    : "";
  const pattern = config["group-pull-request-title-pattern"] ||
    "chore${scope}: release${component} ${version}";
  const title = pattern
    .replace("${scope}", "(main)")
    .replace("${component}", componentText)
    .replace("${version}", version)
    .trim();
  if (title.includes("${")) {
    throw new Error(`Unsupported Release Please title pattern: ${pattern}`);
  }
  return title;
}

export function releasePullRequestFor(pullRequests, version, config, mergeCommitSha) {
  const title = releasePullRequestTitle(config, version);
  return pullRequests.find(
    (pullRequest) =>
      pullRequest.base?.ref === "main" &&
      pullRequest.merged_at &&
      pullRequest.merge_commit_sha === mergeCommitSha &&
      pullRequest.title === title &&
      pullRequest.labels?.some((label) => label.name === "autorelease: tagged"),
  );
}

export function releaseTagForVersion(version) {
  return `v${version}`;
}

export function resolveProductionRelease({
  sourceSha,
  repository,
  checkoutRoot = defaultRoot,
  commandEnv = process.env,
  output = process.env.GITHUB_OUTPUT,
} = {}) {
  if (!/^[A-Za-z0-9_.-]+\/[A-Za-z0-9_.-]+$/.test(repository || "")) {
    throw new Error(`Invalid GITHUB_REPOSITORY: ${repository || "<empty>"}`);
  }
  if (!/^[0-9a-f]{40}$/.test(sourceSha || "")) {
    throw new Error(`Invalid release source SHA: ${sourceSha || "<empty>"}`);
  }
  const state = verifyReleaseVersions(checkoutRoot);
  const headSha = command("git", ["rev-parse", "HEAD"], {
    cwd: checkoutRoot,
    env: commandEnv,
  });
  if (sourceSha !== headSha) {
    throw new Error(`Checked out ${headSha}, expected release source ${sourceSha}`);
  }
  const tag = releaseTagForVersion(state.packageVersion);

  let tagSha;
  try {
    tagSha = command("git", ["rev-list", "-n", "1", `refs/tags/${tag}`], {
      cwd: checkoutRoot,
      env: commandEnv,
      quiet: true,
    });
  } catch {
    writeOutputs(
      { should_release: "false", release_tag: tag, release_version: state.packageVersion, release_sha: headSha },
      output,
    );
    console.log(`No ${tag} tag exists yet; this main update did not create a release.`);
    return false;
  }
  try {
    command("git", ["merge-base", "--is-ancestor", tagSha, headSha], {
      cwd: checkoutRoot,
      env: commandEnv,
      quiet: true,
    });
  } catch {
    throw new Error(`Release tag ${tag} at ${tagSha} is not an ancestor of successful main run ${headSha}`);
  }
  const taggedVersion = JSON.parse(
    command("git", ["show", `${tagSha}:package.json`], {
      cwd: checkoutRoot,
      env: commandEnv,
      quiet: true,
    }),
  ).version;
  if (taggedVersion !== state.packageVersion) {
    throw new Error(
      `Release tag ${tag} contains package version ${taggedVersion}, expected ${state.packageVersion}`,
    );
  }
  const newestTag = command(
    "git",
    ["tag", "--list", "v[0-9]*.[0-9]*.[0-9]*", "--sort=-version:refname"],
    { cwd: checkoutRoot, env: commandEnv, quiet: true },
  ).split("\n")[0];
  if (newestTag !== tag) {
    throw new Error(`Release ${tag} is superseded by newer version tag ${newestTag}`);
  }

  let release;
  try {
    release = JSON.parse(
      command(
        "gh",
        ["release", "view", tag, "--repo", repository, "--json", "isDraft,isPrerelease,tagName,assets"],
        { cwd: checkoutRoot, env: commandEnv, quiet: true },
      ),
    );
  } catch {
    throw new Error(`Release Please did not create the expected draft release ${tag}`);
  }
  if (release.tagName !== tag || release.isPrerelease) {
    throw new Error(`Release ${tag} has unexpected GitHub release metadata`);
  }

  const currentReleasePleaseConfig = JSON.parse(
    command("git", ["show", `${headSha}:release-please-config.json`], {
      cwd: checkoutRoot,
      env: commandEnv,
      quiet: true,
    }),
  );
  const isBootstrapRelease = currentReleasePleaseConfig["bootstrap-sha"] === tagSha;

  // Only the explicitly configured bootstrap release predates Release Please
  // provenance. It is already public and immutable from this workflow, but an
  // incomplete asset set must still fail closed.
  if (!release.isDraft && isBootstrapRelease) {
    if (!completePublishedAssetSet(release.assets || [], state.packageVersion)) {
      throw new Error(`Published release ${tag} is missing required macOS or updater assets`);
    }
    writeOutputs(
      { should_release: "false", release_tag: tag, release_version: state.packageVersion, release_sha: tagSha },
      output,
    );
    console.log(`${tag} is already published with a complete asset set.`);
    return false;
  }

  const releasePleaseConfig = JSON.parse(
    command("git", ["show", `${tagSha}:release-please-config.json`], {
      cwd: checkoutRoot,
      env: commandEnv,
      quiet: true,
    }),
  );

  const pullRequests = JSON.parse(
    command(
      "gh",
      [
        "api",
        "-H",
        "Accept: application/vnd.github+json",
        `repos/${repository}/commits/${tagSha}/pulls`,
      ],
      { cwd: checkoutRoot, env: commandEnv, quiet: true },
    ),
  );
  const releasePullRequest = releasePullRequestFor(
    pullRequests,
    state.packageVersion,
    releasePleaseConfig,
    tagSha,
  );
  if (!releasePullRequest) {
    throw new Error(
      `${tag} is not backed by the merged, autorelease: tagged Release Please PR for this version`,
    );
  }

  if (!release.isDraft) {
    if (!completePublishedAssetSet(release.assets || [], state.packageVersion)) {
      throw new Error(`Published release ${tag} is missing required macOS or updater assets`);
    }
    writeOutputs(
      { should_release: "false", release_tag: tag, release_version: state.packageVersion, release_sha: tagSha },
      output,
    );
    console.log(`${tag} is already published with a complete asset set.`);
    return false;
  }

  writeOutputs(
    { should_release: "true", release_tag: tag, release_version: state.packageVersion, release_sha: tagSha },
    output,
  );
  console.log(
    `Validated pending Release Please draft ${tag} at ${tagSha} from successful main run ${headSha}.`,
  );
  return true;
}

if (process.argv[1] && import.meta.url === new URL(`file://${process.argv[1]}`).href) {
  try {
    resolveProductionRelease({
      sourceSha: process.env.RELEASE_SOURCE_SHA || "",
      repository: process.env.GITHUB_REPOSITORY,
    });
  } catch (error) {
    console.error(`release preflight: ${error.message}`);
    process.exitCode = 1;
  }
}
