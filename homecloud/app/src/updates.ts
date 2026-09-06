/**
 * Finding out whether there is a newer HomeCloud, from GitHub's own releases.
 *
 * The same approach as the sibling apps: a small side project does not need
 * update-manifest infrastructure to answer "is there a newer version", and a
 * public repo's release list needs no authentication.
 *
 * The one difference that matters here: every app in this monorepo publishes
 * into the *same* repository, so `/releases/latest` answers with whichever app
 * shipped most recently — RecPocket, YT Pocket, Vault Explorer. The list is
 * fetched and filtered by tag prefix instead, which is the only way to get
 * this app's latest rather than somebody else's.
 */

/**
 * Both names, because the repository is mid-rename. GitHub redirects a renamed
 * repo's API, but a shipped build that only knows one name depends on that
 * redirect surviving — and if the old name is ever claimed by someone else it
 * would start pointing at a stranger's releases. Trying both removes the
 * ordering problem: this build works before the rename and after it.
 */
const REPOS = ["lucasmoy-dev/personal-projects", "lucasmoy-dev/vaultexplorer"];

/** Tags look like `homecloud-v0.5.0`. */
const TAG_PREFIX = "homecloud-v";

export interface Release {
  version: string;
  notes: string;
  pageUrl: string;
  /** The Debian package for this desktop, when the release carries one. */
  debUrl: string | null;
}

interface GitHubAsset {
  name: string;
  browser_download_url: string;
}

interface GitHubRelease {
  tag_name?: string;
  body?: string;
  html_url?: string;
  draft?: boolean;
  prerelease?: boolean;
  assets?: GitHubAsset[];
}

/** The newest HomeCloud release, or `null` when none has been published. */
export async function latestRelease(): Promise<Release | null> {
  const releases = await fetchReleases();
  const ours = releases
    .filter((release) => !release.draft && !release.prerelease)
    .filter((release) => (release.tag_name ?? "").startsWith(TAG_PREFIX))
    .map(toRelease)
    // GitHub returns newest first, but by publication date, and a re-published
    // older version would then win. Version order is what actually matters.
    .sort((a, b) => compareVersions(b.version, a.version));
  return ours[0] ?? null;
}

async function fetchReleases(): Promise<GitHubRelease[]> {
  let lastError: unknown = new Error("no se pudo consultar GitHub");
  for (const repo of REPOS) {
    try {
      const response = await fetch(`https://api.github.com/repos/${repo}/releases?per_page=30`, {
        headers: { Accept: "application/vnd.github+json" },
      });
      if (!response.ok) throw new Error(`GitHub respondió ${response.status}`);
      return (await response.json()) as GitHubRelease[];
    } catch (error) {
      lastError = error;
    }
  }
  throw lastError;
}

function toRelease(release: GitHubRelease): Release {
  const tag = release.tag_name ?? "";
  const deb = (release.assets ?? []).find((asset) => asset.name.toLowerCase().endsWith(".deb"));
  return {
    version: tag.slice(TAG_PREFIX.length),
    notes: release.body ?? "",
    pageUrl: release.html_url ?? `https://github.com/${REPOS[1]}/releases`,
    debUrl: deb?.browser_download_url ?? null,
  };
}

/** Positive when `a` is newer, negative when older, zero when the same. */
export function compareVersions(a: string, b: string): number {
  const left = parts(a);
  const right = parts(b);
  for (let i = 0; i < Math.max(left.length, right.length); i++) {
    const difference = (left[i] ?? 0) - (right[i] ?? 0);
    if (difference !== 0) return difference;
  }
  return 0;
}

export function isNewer(candidate: string, current: string): boolean {
  return compareVersions(candidate, current) > 0;
}

function parts(version: string): number[] {
  return version
    .replace(/^v/, "")
    .split(".")
    .map((piece) => Number.parseInt(piece, 10) || 0);
}
