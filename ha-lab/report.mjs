import fs from 'node:fs';
import os from 'node:os';
import path from 'node:path';

const file = process.argv[2] ?? path.join(os.homedir(), '.cache/harvest-ha-lab/report.json');
const report = JSON.parse(fs.readFileSync(file, 'utf8'));

const rows = report.results.map((r, i) => {
  const { name, passed, ...details } = r;
  const facts = Object.entries(details)
    .filter(([k]) => k !== 'failures')
    .map(([k, v]) => `${k.replaceAll('_', ' ')}: ${Array.isArray(v) ? v.join(', ') : v}`)
    .join('; ');
  return `| ${i + 1} | ${name} | ${passed ? 'pass' : 'FAIL'} | ${facts} |`;
});

console.log(`HA lab run ${report.started ?? ''} → ${report.finished ?? ''}`);
console.log('');
console.log('| # | Scenario | Result | Measurements |');
console.log('|---|---|---|---|');
console.log(rows.join('\n'));
console.log('');
console.log(`${report.passed ?? report.results.filter(r => r.passed).length} passed, ${report.failed ?? report.results.filter(r => !r.passed).length} failed`);
