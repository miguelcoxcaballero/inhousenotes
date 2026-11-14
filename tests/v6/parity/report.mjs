// Writes the parity report: REPORT.md, results.json and, per failing or warned capture, the two screenshots, the
// pixel diff and the DOM / canvas-trace diffs that locate the cause (TESTING §3.2, §3.5).
import fs from 'node:fs';
import path from 'node:path';

const safe = text => String(text).replace(/[^A-Za-z0-9._@-]+/g, '_');

/**
 * @param {string} outDir
 * @param {Array<object>} results one entry per capture comparison:
 *   {state, cell, capture, kind: 'parity'|'stability', status: 'pass'|'fail'|'warn'|'expected'|'error', tags, message,
 *    comparison?, a?, b?}
 * @param {object} meta {title, a, b, quick, runs, startedAt, durationMs, saveAll: also write the screenshots of passing captures}
 */
export function writeReport(outDir, results, meta) {
  fs.rmSync(outDir, { recursive: true, force: true });
  fs.mkdirSync(outDir, { recursive: true });
  const count = status => results.filter(r => r.status === status).length;
  const rows = [];
  for (const r of results) {
    if (r.status === 'pass' && !(meta.saveAll && r.kind === 'parity')) continue;
    const dir = path.join(outDir, safe(r.state), `${safe(r.cell)}${r.capture ? '.' + safe(r.capture) : ''}${r.kind === 'stability' ? '.stability-' + safe(r.sides || '') : ''}`);
    const files = [];
    if (r.a?.png || r.comparison) {
      fs.mkdirSync(dir, { recursive: true });
      const put = (name, data) => { if (data == null) return; fs.writeFileSync(path.join(dir, name), data); files.push(path.relative(outDir, path.join(dir, name))); };
      put('a.png', r.a?.png); put('b.png', r.b?.png);
      if (r.comparison && !r.comparison.pixelsEqual) put('diff.png', r.comparison.pixels.diff());
      put('dom.diff', r.comparison?.dom); put('trace.diff', r.comparison?.trace);
      if (r.comparison?.dom) { put('a.dom.txt', r.a?.dom); put('b.dom.txt', r.b?.dom); }
    }
    if (r.status !== 'pass') rows.push({ ...r, files });
  }
  const line = r => {
    const px = r.comparison && !r.comparison.pixelsEqual ? (r.comparison.pixels.different < 0 ? `size ${r.comparison.pixels.sizes.join(' vs ')}` : `${r.comparison.pixels.different} px differ, box ${JSON.stringify(r.comparison.pixels.box)}, max Δ ${r.comparison.pixels.maxDelta}`) : 'pixels equal';
    const extra = [r.comparison?.dom ? 'DOM differs' : '', r.comparison?.trace ? 'canvas trace differs' : ''].filter(Boolean).join(', ');
    return `| ${r.status} | ${r.kind}${r.sides ? ' ' + r.sides : ''} | \`${r.state}\`${r.capture ? ' @' + r.capture : ''} | ${r.cell} | ${(r.tags || []).join(' ') || '–'} | ${r.message || [px, extra].filter(Boolean).join('; ')} | ${r.files.map(f => `[${path.basename(f)}](${f})`).join(' ')} |`;
  };
  const section = (title, status, note) => {
    const list = rows.filter(r => r.status === status);
    return list.length ? `\n## ${title} (${list.length})\n\n${note}\n\n| Result | Check | State | Cell | Tags | Detail | Files |\n|---|---|---|---|---|---|---|\n${list.map(line).join('\n')}\n` : '';
  };
  const problems = [...new Set(results.flatMap(r => (r.problems || []).map(p => `- \`${r.state}\` [${r.cell}] ${p}`)))];
  const md = `# Pixel-parity report — ${meta.title}

- A: ${meta.a}
- B: ${meta.b}
- Matrix: ${meta.quick ? 'quick (desktop 1280×800 + phone 360×800, light, web)' : 'full'}; runs: ${meta.runs}; tolerance 0
- Started ${meta.startedAt}; ${(meta.durationMs / 1000).toFixed(1)} s

| Comparisons | Pass | Fail | Error | Expected (BEHAVIOUR-CHANGES) | Warnings (pixels equal, DOM/trace differ) |
|---|---|---|---|---|---|
| ${results.length} | ${count('pass')} | ${count('fail')} | ${count('error')} | ${count('expected')} | ${count('warn')} |

**${count('fail') + count('error') === 0 ? 'PASS' : 'FAIL'}**
${section('Failures', 'fail', 'A screenshot difference outside a BEHAVIOUR-CHANGES rule. `stability` rows compare two runs of the same app.')}${section('Errors', 'error', 'The state script did not finish on one side.')}${section('Expected differences', 'expected', 'States tagged with a BEHAVIOUR-CHANGES id whose rule matched; for the user\'s review (TESTING §3.5).')}${section('Warnings', 'warn', 'Pixels match; the DOM or the canvas trace differs. Not a failure; W6.1 looks at these.')}${problems.length ? `\n## Page problems (${problems.length})\n\nPage errors, CSP violations, requests to unmocked hosts, pages that never went idle. Diagnostics, not failures.\n\n${problems.join('\n')}\n` : ''}`;
  fs.writeFileSync(path.join(outDir, 'REPORT.md'), md);
  fs.writeFileSync(path.join(outDir, 'results.json'), JSON.stringify(results.map(r => ({ state: r.state, cell: r.cell, capture: r.capture, kind: r.kind, sides: r.sides, status: r.status, tags: r.tags, message: r.message,
    pixels: r.comparison ? { equal: r.comparison.pixelsEqual, different: r.comparison.pixels.different, box: r.comparison.pixels.box } : undefined, dom: r.comparison ? !r.comparison.dom : undefined, trace: r.comparison ? !r.comparison.trace : undefined })), null, 1));
  return { failed: count('fail') + count('error'), passed: count('pass'), warned: count('warn'), expected: count('expected'), file: path.join(outDir, 'REPORT.md') };
}

/**
 * `node tests/v6/parity/report.mjs --summary <run dir>… [--out tests/v6/parity/REPORT.md] [--title text]`: one
 * report over several runs (the matrix run file by file): totals per run and every non-passing comparison, read from
 * each run's results.json (the screenshots and diffs stay in the run directories).
 */
export function writeSummary(dirs, out, title = 'parity summary') {
  const runs = dirs.map(dir => {
    const file = path.join(dir, 'results.json');
    const results = fs.existsSync(file) ? JSON.parse(fs.readFileSync(file, 'utf8')) : null;
    const head = fs.existsSync(path.join(dir, 'REPORT.md')) ? fs.readFileSync(path.join(dir, 'REPORT.md'), 'utf8') : '';
    return { name: path.basename(dir), results, matrix: (/- Matrix: ([^;\n]*)/.exec(head) || [])[1] || '?' };
  });
  const all = runs.flatMap(r => (r.results || []).map(x => ({ ...x, run: r.name })));
  const n = (list, status) => list.filter(r => r.status === status).length;
  const rowsOf = status => all.filter(r => r.status === status).map(r => `| ${r.run} | \`${r.state}\`${r.capture ? ' @' + r.capture : ''} | ${r.cell} | ${(r.tags || []).join(' ') || '–'} | ${r.message || (r.pixels && !r.pixels.equal ? `${r.pixels.different} px, box ${JSON.stringify(r.pixels.box)}` : '')} |`);
  const table = (title, status) => { const rows = rowsOf(status); return rows.length ? `\n## ${title} (${rows.length})\n\n| Run | State | Cell | Tags | Detail |\n|---|---|---|---|---|\n${rows.join('\n')}\n` : ''; };
  const md = `# Pixel-parity report — ${title}

Generated by \`node tests/v6/parity/report.mjs --summary\` from ${runs.length} runs (one per state file).

| Run | Matrix | Comparisons | Pass | Fail | Error | Expected | Warnings |
|---|---|---|---|---|---|---|---|
${runs.map(r => r.results ? `| ${r.name} | ${r.matrix} | ${r.results.length} | ${n(r.results, 'pass')} | ${n(r.results, 'fail')} | ${n(r.results, 'error')} | ${n(r.results, 'expected')} | ${n(r.results, 'warn')} |` : `| ${r.name} | – | not run | | | | | |`).join('\n')}
| **all** | | ${all.length} | ${n(all, 'pass')} | ${n(all, 'fail')} | ${n(all, 'error')} | ${n(all, 'expected')} | ${n(all, 'warn')} |

**${n(all, 'fail') + n(all, 'error') === 0 ? 'PASS' : 'FAIL'}**
${table('Failures', 'fail')}${table('Errors', 'error')}${table('Expected differences (BEHAVIOUR-CHANGES)', 'expected')}`;
  fs.writeFileSync(out, md);
  return out;
}

if (process.argv[1] && path.resolve(process.argv[1]) === path.resolve(new URL(import.meta.url).pathname) && process.argv.includes('--summary')) {
  const args = process.argv.slice(2), dirs = [];
  let out = path.join(path.dirname(new URL(import.meta.url).pathname), 'REPORT.md'), title;
  for (let i = 0; i < args.length; i++) {
    if (args[i] === '--out') out = args[++i];
    else if (args[i] === '--title') title = args[++i];
    else if (args[i] !== '--summary') dirs.push(args[i]);
  }
  console.log(writeSummary(dirs, out, title));
}
