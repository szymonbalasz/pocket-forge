#!/usr/bin/env node
// Reads Playwright's JSON report and flags tests that passed only on retry (AGENTS §5: report
// them as flaky and add a backlog row). Annotates the CI run; never fails it by itself.
import { existsSync, readFileSync } from 'node:fs';

const file = process.argv[2] ?? 'app/test-results/results.json';
if (!existsSync(file)) {
  console.log(`report-flaky: no report at ${file}`);
  process.exit(0);
}
const report = JSON.parse(readFileSync(file, 'utf8'));
const flaky = [];
const walk = (suite, path) => {
  for (const s of suite.suites ?? []) walk(s, [...path, s.title]);
  for (const spec of suite.specs ?? []) {
    for (const t of spec.tests ?? []) {
      if (t.status === 'flaky') flaky.push([...path, spec.title].filter(Boolean).join(' › '));
    }
  }
};
for (const s of report.suites ?? []) walk(s, [s.title]);
for (const name of flaky) console.log(`::warning title=Flaky test::${name} passed only on retry. Add a backlog row (AGENTS §5).`);
console.log(`report-flaky: ${flaky.length} flaky test(s)`);
