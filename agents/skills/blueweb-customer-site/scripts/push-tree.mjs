#!/usr/bin/env node
// Push the local HEAD commit's tree to a GitHub branch through the REST
// git-data API, using `gh api` for auth. For where `git push` cannot work:
// the Managed Agents sandbox substitutes the real token only into request
// *headers*, and git-over-HTTPS sends it base64-encoded in Basic auth, so
// every `git push` there fails with `remote: invalid credentials`.
//
//   node scripts/push-tree.mjs <owner/repo> <branch> [-m "message"] [--from <branch>]
//
// What it does, and why it is safe to trust:
//
// - Every blob is uploaded from `git cat-file`, base64, with its git mode.
//   No file content passes through the model, so a 143 KB lockfile costs
//   nothing in tokens and cannot be mistranscribed.
// - GitHub's tree SHA is compared with `git rev-parse HEAD^{tree}`. Git
//   tree ids are content-addressed, so equality proves every byte and mode
//   on GitHub equals the local commit. A mismatch is a hard failure.
// - The commit is created with the branch's current head as its parent
//   (fast-forward only), or from `--from` (default: the repo's default
//   branch) when the branch does not exist yet.
// - An empty repository is seeded with one file through the contents API
//   first, because the git-data API refuses to create trees on a repo with
//   no commits (409). The seed commit becomes the parent.
//
// Requires: git, node, and an authenticated `gh` (`gh auth status`).
// The remote commit's SHA is printed; GitHub is canonical after this, so
// `git fetch` is not needed but `git log origin/<branch>` will not show it.

import { execFileSync } from 'node:child_process';

const args = process.argv.slice(2);
const positional = [];
let message = null;
let from = null;
for (let i = 0; i < args.length; i++) {
  if (args[i] === '-m' || args[i] === '--message') message = args[++i];
  else if (args[i] === '--from') from = args[++i];
  else if (args[i] === '-h' || args[i] === '--help') usage(0);
  else positional.push(args[i]);
}
const [repo, branch] = positional;
if (!repo || !branch || !/^[^/\s]+\/[^/\s]+$/.test(repo)) usage(1);

function usage(code) {
  console.error('usage: push-tree.mjs <owner/repo> <branch> [-m "message"] [--from <branch>]');
  process.exit(code);
}

function git(...a) {
  return execFileSync('git', a, { encoding: 'utf8', maxBuffer: 64 << 20 }).trimEnd();
}
function gitBytes(...a) {
  return execFileSync('git', a, { maxBuffer: 256 << 20 });
}

// `gh api` with a JSON body on stdin. Returns the parsed response; throws
// with GitHub's message on a non-2xx. With `allow404`, "no such ref" is
// answered with `null` instead — that is a 404 on a repo with commits and
// a `409 Git Repository is empty` on one without.
function gh(method, path, body, { allow404 = false } = {}) {
  const a = ['api', path, '--method', method];
  if (body !== undefined) a.push('--input', '-');
  try {
    const out = execFileSync('gh', a, {
      input: body === undefined ? undefined : JSON.stringify(body),
      encoding: 'utf8',
      maxBuffer: 64 << 20,
      stdio: ['pipe', 'pipe', 'pipe'],
    });
    return out ? JSON.parse(out) : null;
  } catch (e) {
    const text = `${e.stdout ?? ''}${e.stderr ?? ''}`;
    if (allow404 && /HTTP 404|Not Found|Git Repository is empty/.test(text)) return null;
    throw new Error(`gh api ${method} ${path}: ${text.trim() || e.message}`);
  }
}

// ---- local side: what HEAD is ---------------------------------------------

const head = git('rev-parse', 'HEAD');
const localTree = git('rev-parse', 'HEAD^{tree}');
if (git('status', '--porcelain')) {
  console.error('note: working tree has uncommitted changes; pushing HEAD, not the working tree');
}
const entries = git('ls-tree', '-r', 'HEAD')
  .split('\n')
  .filter(Boolean)
  .map((line) => {
    const [meta, path] = line.split('\t');
    const [mode, type, sha] = meta.split(/\s+/);
    return { mode, type, sha, path };
  });
const submodule = entries.find((e) => e.type !== 'blob');
if (submodule) {
  console.error(`cannot push ${submodule.path}: ${submodule.type} entries are not supported`);
  process.exit(1);
}
if (message === null) message = git('log', '-1', '--format=%B', 'HEAD').trim();

// ---- remote side: where the branch is ------------------------------------

const ref = gh('GET', `repos/${repo}/git/ref/heads/${branch}`, undefined, { allow404: true });
let parent = ref?.object?.sha ?? null;
let branchExists = parent !== null;

if (!branchExists) {
  const base = from ?? gh('GET', `repos/${repo}`).default_branch;
  const baseRef = gh('GET', `repos/${repo}/git/ref/heads/${base}`, undefined, { allow404: true });
  if (baseRef) {
    parent = baseRef.object.sha;
    console.error(`branch ${branch} does not exist; branching from ${base} @ ${parent.slice(0, 7)}`);
  } else {
    // Empty repository. Seed it with the smallest file so the git-data API
    // will talk to us; that commit is the parent of the real one.
    const seed = [...entries].sort((a, b) => a.path.length - b.path.length || a.path.localeCompare(b.path))
      .find((e) => e.path === '.nvmrc') ?? entries[0];
    console.error(`repository is empty; seeding ${branch} with ${seed.path}`);
    const res = gh('PUT', `repos/${repo}/contents/${seed.path}`, {
      message: `Seed ${branch}`,
      content: gitBytes('cat-file', 'blob', seed.sha).toString('base64'),
      branch,
    });
    parent = res.commit.sha;
    branchExists = true;
  }
}

// ---- blobs, tree, commit, ref ---------------------------------------------

console.error(`uploading ${entries.length} blobs to ${repo}…`);
for (const e of entries) {
  const res = gh('POST', `repos/${repo}/git/blobs`, {
    content: gitBytes('cat-file', 'blob', e.sha).toString('base64'),
    encoding: 'base64',
  });
  if (res.sha !== e.sha) {
    console.error(`blob mismatch for ${e.path}: local ${e.sha}, GitHub ${res.sha}`);
    process.exit(1);
  }
}

const tree = gh('POST', `repos/${repo}/git/trees`, {
  tree: entries.map((e) => ({ path: e.path, mode: e.mode, type: 'blob', sha: e.sha })),
});
if (tree.sha !== localTree) {
  console.error(`tree mismatch: local HEAD^{tree} ${localTree}, GitHub ${tree.sha} — nothing was committed`);
  process.exit(1);
}

const commit = gh('POST', `repos/${repo}/git/commits`, {
  message,
  tree: tree.sha,
  parents: parent ? [parent] : [],
});

if (branchExists) {
  gh('PATCH', `repos/${repo}/git/refs/heads/${branch}`, { sha: commit.sha, force: false });
} else {
  gh('POST', `repos/${repo}/git/refs`, { ref: `refs/heads/${branch}`, sha: commit.sha });
}

console.log(`pushed ${repo}@${branch}`);
console.log(`  commit ${commit.sha}  (local ${head.slice(0, 7)}: same tree, new commit object)`);
console.log(`  tree   ${tree.sha}  matches local HEAD^{tree} — byte-identical`);
console.log(`  ${entries.length} files`);
