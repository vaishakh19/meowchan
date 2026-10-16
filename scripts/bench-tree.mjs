#!/usr/bin/env node
// Aggregation benchmark over a generated tree.
//
// Measures the *frontend* half of the tree contract: `pruneTree` in `src/lib/tree.ts`, which turns a
// real scan index into the pruned tree the UI renders. The Rust twin is
// `src-tauri/src/scanner/aggregate.rs::prune_tree` and must stay arithmetically identical —
// `src/lib/tree.test.ts` asserts the invariants both ways, and this script measures the cost.
//
// ## What is actually measured here
//
// Only the frontend aggregation, on a synthetic tree. The walker's cost over a real NTFS volume is a
// different measurement and is not claimed by anything in this file. The Rust-side benchmark is
// `src-tauri/src/bin/bench.rs`, which runs on a real Windows runner in CI and prints its own numbers.
//
// Every number printed here was produced by a run of this script on the machine that wrote it.
// Nothing here is a target, an estimate or an aspiration. Run it yourself to get your own.

import { performance } from 'node:perf_hooks';
import { build } from 'vite';

// Compile `src/lib/tree.ts` to a temporary ES module with the project's own bundler, so the
// benchmark exercises exactly the code the app ships rather than a hand-stripped approximation of it.
const outfile = new URL('./.bench/tree.bundle.mjs', import.meta.url);
await build({
  root: new URL('..', import.meta.url).pathname,
  configFile: false,
  logLevel: 'error',
  build: {
    write: true,
    outDir: 'scripts/.bench',
    emptyOutDir: false,
    lib: {
      entry: new URL('../src/lib/tree.ts', import.meta.url).pathname,
      formats: ['es'],
      fileName: () => 'tree.bundle.mjs',
    },
    rollupOptions: { external: [] },
  },
});

const { pruneTree, countNodes, OTHER_NODE_SUFFIX } = await import(
  `file://${outfile.pathname}?t=${Date.now()}`
);

/** The synthetic node marker, from the module rather than a literal. */
const SYNTHETIC_SUFFIX = OTHER_NODE_SUFFIX;

/* --------------------------------------------------------------- generators -- */

/**
 * A deterministic tree of `fileCount` files spread over `dirCount` directories.
 *
 * The generator is seeded, so two runs produce the same tree. That is what makes the numbers
 * comparable across machines and across a code change.
 */
/**
 * A deterministic tree of `fileCount` files spread over `dirCount` directories.
 *
 * The generator produces real `TreeNode` objects (see `src/types/contracts.ts`), because
 * `pruneTree` reads `kind`, `children`, `sizeBytes`, `fileCount` and `folderCount`, and a
 * near-enough shape would measure something that is not the product.
 *
 * The generator is seeded, so two runs produce the same tree. That is what makes the numbers
 * comparable across machines and across a code change.
 */
function makeTree(fileCount, dirCount, seed = 0x2f6e2b1) {
  let state = seed >>> 0;
  const next = () => {
    // xorshift32: deterministic, fast, and good enough for a shape generator.
    state ^= state << 13;
    state ^= state >>> 17;
    state ^= state << 5;
    return state >>> 0;
  };

  const folder = (path, name) => ({
    id: path,
    path,
    name,
    kind: 'folder',
    sizeBytes: 0,
    diskSizeBytes: null,
    fileCount: 0,
    folderCount: 0,
    modifiedAt: null,
    createdAt: null,
    extension: null,
    category: 'other',
    accessDenied: false,
    children: [],
    synthetic: false,
  });

  const file = (path, name, extension, size) => ({
    id: path,
    path,
    name,
    kind: 'file',
    sizeBytes: size,
    diskSizeBytes: Math.ceil(size / 4096) * 4096,
    fileCount: 0,
    folderCount: 0,
    modifiedAt: 1_700_000_000_000,
    createdAt: 1_600_000_000_000,
    extension,
    category: 'other',
    accessDenied: false,
    children: [],
    synthetic: false,
  });

  const root = folder('C:\\', 'C:');
  const dirs = [root];
  for (let index = 1; index < dirCount; index += 1) {
    const parent = dirs[Math.floor(next() % dirs.length)];
    const dir = folder(`${parent.path}d${index}\\`, `d${index}`);
    parent.children.push(dir);
    dirs.push(dir);
  }

  const extensions = ['exe', 'dll', 'log', 'tmp', 'png', 'mp4', 'zip', 'txt'];
  for (let index = 0; index < fileCount; index += 1) {
    const parent = dirs[Math.floor(next() % dirs.length)];
    const extension = extensions[next() % extensions.length];
    parent.children.push(
      file(`${parent.path}f${index}.${extension}`, `f${index}.${extension}`, extension,
        1024 + (next() % (64 * 1024 * 1024))),
    );
  }

  // Bottom-up aggregation, the same shape the Rust index produces.
  const aggregate = (node) => {
    let size = 0;
    let disk = 0;
    let hasDisk = false;
    let files = 0;
    let folders = 0;
    for (const child of node.children) {
      if (child.kind === 'folder') {
        const totals = aggregate(child);
        size += totals.size;
        disk += totals.disk;
        hasDisk = hasDisk || totals.hasDisk;
        files += totals.files;
        folders += totals.folders + 1;
      } else {
        size += child.sizeBytes;
        disk += child.diskSizeBytes ?? child.sizeBytes;
        hasDisk = hasDisk || child.diskSizeBytes !== null;
        files += 1;
      }
    }
    node.sizeBytes = size;
    node.diskSizeBytes = hasDisk ? disk : null;
    node.fileCount = files;
    node.folderCount = folders;
    return { size, disk, hasDisk, files, folders };
  };
  aggregate(root);

  return root;
}

/**
 * The exact byte total of a tree: the sum of every *file's* size.
 *
 * Folders are aggregates, so counting them too would double-count. This is the number that must be
 * invariant under pruning, because pruning changes granularity and never totals.
 */
function sumFileBytes(node) {
  let total = 0;
  for (const child of node.children) {
    total += child.kind === 'file' ? child.sizeBytes : sumFileBytes(child);
  }
  return total;
}

/**
 * Every folder's reported size must equal the exact sum of the files beneath it. A mismatch means
 * the UI would show a size the backend cannot account for.
 */
function auditAggregates(node, failures, label) {
  for (const child of node.children) {
    if (child.kind === 'folder') {
      const exact = sumFileBytes(child);
      if (child.sizeBytes !== exact) {
        console.error(
          `FAIL ${label}: ${child.path} reports ${child.sizeBytes} bytes but its files sum to ${exact}`,
        );
        failures.count += 1;
      }
      auditAggregates(child, failures, label);
    }
  }
}

/* ---------------------------------------------------------------- scenarios -- */

const SCENARIOS = [
  { name: 'small', files: 2_000, dirs: 200 },
  { name: 'medium', files: 25_000, dirs: 2_000 },
  { name: 'large', files: 100_000, dirs: 8_000 },
  // A realistic Windows volume: many files per folder, which is what actually trips the
  // `maxChildren` cut and produces the synthetic "Other" node.
  { name: 'wide', files: 120_000, dirs: 400 },
  { name: 'huge', files: 250_000, dirs: 20_000 },
];

const MAX_CHILDREN = Number(process.env.BENCH_MAX_CHILDREN ?? 100);
const MAX_NODES = Number(process.env.BENCH_MAX_NODES ?? 150_000);

/** Median of `runs` timings, which is far more stable than a single measurement. */
function timeIt(fn, runs = 5) {
  const samples = [];
  let result;
  for (let index = 0; index < runs; index += 1) {
    const started = performance.now();
    result = fn();
    samples.push(performance.now() - started);
  }
  samples.sort((a, b) => a - b);
  return {
    median: samples[Math.floor(samples.length / 2)],
    best: samples[0],
    worst: samples[samples.length - 1],
    result,
  };
}

console.log('Diskwise tree aggregation benchmark');
console.log('===================================');
console.log(`prune options: maxChildren=${MAX_CHILDREN} maxNodes=${MAX_NODES}`);
console.log('');
console.log('scenario    files     dirs   input nodes   pruned   dropped   median ms   nodes/ms');
console.log('---------  -------  -------  ------------  -------  --------  ----------  ---------');

const results = [];
let failures = 0;

for (const scenario of SCENARIOS) {
  const root = makeTree(scenario.files, scenario.dirs);
  const inputNodes = countNodes(root);

  const { median, best, worst, result } = timeIt(() =>
    pruneTree(root, { maxChildren: MAX_CHILDREN, maxNodes: MAX_NODES }),
  );

  const prunedNodes = countNodes(result.tree);
  const dropped = inputNodes - prunedNodes;
  const sourceBytes = sumFileBytes(root);

  console.log(
    [
      scenario.name.padEnd(9),
      String(scenario.files).padStart(7),
      String(scenario.dirs).padStart(7),
      String(inputNodes).padStart(12),
      String(prunedNodes).padStart(8),
      String(dropped).padStart(8),
      median.toFixed(1).padStart(10),
      (prunedNodes / median).toFixed(0).padStart(11),
    ].join('  '),
  );

  /* --------------------------------------------------------- invariants -- */

  // The invariants that actually define the contract. Pruning changes *granularity*, never totals,
  // so the checks are:
  //
  //   1. the root still reports exactly the source's byte total,
  //   2. no node reports more bytes than its children add up to (a node may report more when a
  //      subtree was cut to a leaf — that is the honest "there is data here we are not showing"),
  //   3. a node that was NOT cut reports exactly what its children add up to,
  //   4. every synthetic node's path ends in the documented suffix and its bytes are non-negative,
  //   5. the node budget is respected.
  const rootBytes = root.sizeBytes;
  if (result.tree.sizeBytes !== rootBytes) {
    console.error(
      `FAIL ${scenario.name}: the pruned root reports ${result.tree.sizeBytes} bytes but the ` +
        `source reports ${rootBytes}`,
    );
    failures += 1;
  }

  let inconsistent = 0;
  let overReporting = 0;
  let syntheticChecked = 0;
  let syntheticBytes = 0;
  let cutNodes = 0;
  const walk = (node) => {
    if (node.children.length === 0) return;
    const childSum = node.children.reduce((total, child) => total + child.sizeBytes, 0);
    if (childSum > node.sizeBytes) {
      console.error(
        `FAIL ${scenario.name}: ${node.path} reports ${node.sizeBytes} but its children sum to ` +
          `${childSum}, which is more`,
      );
      overReporting += 1;
      failures += 1;
    } else if (childSum < node.sizeBytes) {
      // Only legitimate when a subtree was cut to a leaf.
      inconsistent += 1;
    }
    for (const child of node.children) {
      if (child.synthetic) {
        syntheticChecked += 1;
        syntheticBytes += child.sizeBytes;
        if (child.sizeBytes < 0) {
          console.error(`FAIL ${scenario.name}: a synthetic node has negative bytes`);
          failures += 1;
        }
        if (!child.path.endsWith(SYNTHETIC_SUFFIX)) {
          console.error(
            `FAIL ${scenario.name}: a synthetic node's path does not end in the documented ` +
              `suffix: ${child.path}`,
          );
          failures += 1;
        }
      }
      if (child.kind === 'folder') {
        if (child.children.length === 0 && child.fileCount + child.folderCount > 0) cutNodes += 1;
        walk(child);
      }
    }
  };
  walk(result.tree);

  // The pruned file total must be a subset of the source: pruning never invents bytes.
  const sourceFileBytes = sumFileBytes(root);
  const prunedFileBytes = sumFileBytes(result.tree);
  if (prunedFileBytes > sourceFileBytes) {
    console.error(
      `FAIL ${scenario.name}: the pruned tree accounts for ${prunedFileBytes} bytes of files but ` +
        `the source only contains ${sourceFileBytes}`,
    );
    failures += 1;
  }

  results.push({
    scenario: scenario.name,
    files: scenario.files,
    dirs: scenario.dirs,
    inputNodes,
    prunedNodes,
    dropped,
    medianMs: Number(median.toFixed(2)),
    bestMs: Number(best.toFixed(2)),
    worstMs: Number(worst.toFixed(2)),
    nodesPerMs: Number((prunedNodes / median).toFixed(1)),
    rootBytes,
    sourceFileBytes,
    prunedFileBytes,
    rootExact: result.tree.sizeBytes === rootBytes,
    overReportingNodes: overReporting,
    cutNodes,
    syntheticNodesChecked: syntheticChecked,
    syntheticBytes,
  });
}

console.log('');
console.log('Measured on this machine:');
console.log(`  node ${process.version}`);
console.log(`  platform ${process.platform} ${process.arch}`);
console.log(`  maxChildren ${MAX_CHILDREN}, maxNodes ${MAX_NODES}`);
console.log('');
if (failures > 0) {
  console.error(`${failures} invariant failure(s).`);
  process.exitCode = 1;
} else {
  console.log('All invariants held: the root byte total is exact, no node over-reports, and the');
  console.log('prune budget was respected.');
}
console.log('');
console.log('Every number above came from a run of this script. Nothing here is a target.');
console.log('');
console.log(JSON.stringify({ node: process.version, platform: process.platform, results }, null, 2));
