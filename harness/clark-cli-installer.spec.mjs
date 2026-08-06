import assert from 'node:assert/strict';
import { execFileSync, spawnSync } from 'node:child_process';
import crypto from 'node:crypto';
import fs from 'node:fs';
import os from 'node:os';
import path from 'node:path';
import test from 'node:test';

const ROOT = path.resolve(import.meta.dirname, '..');
const INSTALLER = path.join(ROOT, 'scripts', 'install.sh');
const VERSION = '9.8.7';
const TARGET = 'x86_64-unknown-linux-gnu';
const ASSET = `clark-${TARGET}.tar.gz`;

function executable(pathname, body) {
  fs.writeFileSync(pathname, body, { mode: 0o755 });
}

function fixture() {
  const root = fs.mkdtempSync(path.join(os.tmpdir(), 'clark-installer-test-'));
  const source = path.join(root, 'source');
  const release = path.join(source, 'releases', `v${VERSION}`);
  const packageBin = path.join(root, 'package', 'bin');
  const shim = path.join(root, 'shim');
  fs.mkdirSync(release, { recursive: true });
  fs.mkdirSync(packageBin, { recursive: true });
  fs.mkdirSync(path.join(source, 'latest'), { recursive: true });
  fs.mkdirSync(shim, { recursive: true });
  executable(
    path.join(packageBin, 'clark'),
    '#!/bin/sh\n[ "${1:-}" = "--version" ] && { echo "clark 9.8.7"; exit 0; }\nexit 0\n',
  );
  executable(
    path.join(packageBin, 'clark-code-headless'),
    '#!/bin/sh\n[ "${1:-}" = "--self-test" ] && { echo "{\\"status\\":\\"passed\\"}"; exit 0; }\nexit 1\n',
  );
  execFileSync('tar', ['-czf', path.join(release, ASSET), '-C', path.join(root, 'package'), 'bin']);
  const archive = fs.readFileSync(path.join(release, ASSET));
  const digest = crypto.createHash('sha256').update(archive).digest('hex');
  fs.writeFileSync(path.join(release, 'SHA256SUMS'), `${digest}  ${ASSET}\n`);
  fs.writeFileSync(path.join(source, 'latest', 'VERSION'), `${VERSION}\n`);
  executable(
    path.join(shim, 'uname'),
    '#!/bin/sh\ncase "${1:-}" in -s) echo Linux;; -m) echo x86_64;; *) echo Linux;; esac\n',
  );
  return { root, source, release, shim };
}

function install(setup) {
  const clarkHome = path.join(setup.root, 'clark-home');
  const installBin = path.join(setup.root, 'install-bin');
  const result = spawnSync('/bin/sh', [INSTALLER], {
    encoding: 'utf8',
    env: {
      ...process.env,
      CLARK_HOME: clarkHome,
      CLARK_INSTALL_DIR: installBin,
      CLARK_INSTALL_BASE_URL: `file://${setup.source}`,
      PATH: `${setup.shim}:${process.env.PATH}`,
    },
  });
  return { ...setup, clarkHome, installBin, result };
}

test('installs the verified paired CLI and specialist worker atomically', () => {
  const receipt = install(fixture());
  try {
    assert.equal(receipt.result.status, 0, receipt.result.stderr);
    assert.match(receipt.result.stdout, /Installed Clark 9\.8\.7/);
    const clark = path.join(receipt.installBin, 'clark');
    const worker = path.join(receipt.installBin, 'clark-code-headless');
    assert.equal(fs.lstatSync(clark).isSymbolicLink(), true);
    assert.equal(fs.lstatSync(worker).isSymbolicLink(), true);
    assert.equal(execFileSync(clark, ['--version'], { encoding: 'utf8' }).trim(), 'clark 9.8.7');
    assert.match(execFileSync(worker, ['--self-test'], { encoding: 'utf8' }), /passed/);
  } finally {
    fs.rmSync(receipt.root, { recursive: true, force: true });
  }
});

test('refuses a release whose archive does not match its checksum', () => {
  const setup = fixture();
  fs.appendFileSync(path.join(setup.release, ASSET), 'tampered');
  const receipt = install(setup);
  try {
    assert.notEqual(receipt.result.status, 0);
    assert.match(receipt.result.stderr, /checksum mismatch/);
    assert.equal(fs.existsSync(path.join(receipt.installBin, 'clark')), false);
  } finally {
    fs.rmSync(receipt.root, { recursive: true, force: true });
  }
});

test('reinstall is idempotent and refuses a changed installed release', () => {
  const setup = fixture();
  const first = install(setup);
  try {
    assert.equal(first.result.status, 0, first.result.stderr);
    const second = install(setup);
    assert.equal(second.result.status, 0, second.result.stderr);

    fs.appendFileSync(
      path.join(first.clarkHome, 'packages', 'cli', 'releases', VERSION, 'bin', 'clark'),
      'changed',
    );
    const third = install(setup);
    assert.notEqual(third.result.status, 0);
    assert.match(third.result.stderr, /differs from verified/);
  } finally {
    fs.rmSync(setup.root, { recursive: true, force: true });
  }
});
