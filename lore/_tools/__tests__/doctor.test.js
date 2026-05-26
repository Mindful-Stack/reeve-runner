// lore/_tools/__tests__/doctor.test.js
'use strict';
const { test } = require('node:test');
const assert = require('node:assert/strict');
const fs = require('fs');
const os = require('os');
const path = require('path');
const { runDoctor } = require('../doctor');

function withTempWorkspace(callback) {
    const tmp = fs.mkdtempSync(path.join(os.tmpdir(), 'doctor-test-'));
    try { return callback(tmp); } finally { fs.rmSync(tmp, { recursive: true, force: true }); }
}

test('doctor returns 0 for a clean workspace', () => {
    withTempWorkspace((tmp) => {
        fs.writeFileSync(path.join(tmp, 'household.json'), JSON.stringify({
            meta_repo: 'ws',
            repos: [{ name: 'ws' }, { name: 'lore' }]
        }));
        const kbDir = path.join(tmp, 'lore', 'knowledge');
        fs.mkdirSync(path.join(kbDir, 'general'), { recursive: true });
        fs.writeFileSync(path.join(kbDir, 'general', 'hello.md'),
            '---\ntitle: Hello\ndescription: Sample\ntags: [general]\n---\n\n# Hello\n');
        const exitCode = runDoctor(kbDir);
        assert.equal(exitCode, 0);
    });
});

test('doctor returns 1 when manifest workspace pointer is invalid', () => {
    withTempWorkspace((tmp) => {
        fs.writeFileSync(path.join(tmp, 'household.json'), JSON.stringify({
            meta_repo: 'ghost',
            repos: [{ name: 'ws' }]
        }));
        const kbDir = path.join(tmp, 'lore', 'knowledge');
        fs.mkdirSync(kbDir, { recursive: true });
        const exitCode = runDoctor(kbDir);
        assert.equal(exitCode, 1);
    });
});

test('doctor reports missing sibling presence', () => {
    withTempWorkspace((tmp) => {
        fs.writeFileSync(path.join(tmp, 'household.json'), JSON.stringify({
            meta_repo: 'ws',
            repos: [{ name: 'ws' }, { name: 'missing-sibling' }]
        }));
        const kbDir = path.join(tmp, 'lore', 'knowledge');
        fs.mkdirSync(kbDir, { recursive: true });
        const origLog = console.log;
        let captured = '';
        console.log = (msg) => { captured += msg + '\n'; };
        try {
            runDoctor(kbDir);
        } finally {
            console.log = origLog;
        }
        assert.match(captured, /missing-sibling/);
    });
});
