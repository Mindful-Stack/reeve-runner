// lore/_tools/doctor.js
'use strict';

const fs = require('fs');
const path = require('path');
const { validateAll: validateFrontmatter } = require('./validate-frontmatter');
const { validateAll: validateLinks } = require('./validate-links');
const { findOrphans } = require('./check-orphans');

/**
 * Run the full doctor diagnostic. Returns exit code (0 = clean, 1 = errors).
 * @param {string} knowledgeDir - path to the knowledge directory (default: ../knowledge)
 * @param {Object} options - { manifestPath: string }
 */
function runDoctor(knowledgeDir, options = {}) {
    const issues = { error: [], warning: [], info: [] };

    const manifestPath = options.manifestPath || findManifest(knowledgeDir);
    if (manifestPath) {
        checkManifest(manifestPath, issues);
    } else {
        issues.warning.push('No household.json found — workspace checks skipped.');
    }

    if (!fs.existsSync(knowledgeDir)) {
        issues.error.push(`Knowledge dir not found at ${knowledgeDir}`);
        return reportAndExit(issues);
    }
    checkIndexStaleness(knowledgeDir, issues);
    try {
        // validateFrontmatter returns { [relPath]: [errorMsg, ...] }
        const frontmatterErrors = validateFrontmatter(knowledgeDir);
        for (const [file, errs] of Object.entries(frontmatterErrors)) {
            for (const msg of errs) {
                issues.error.push(`Frontmatter: ${file}: ${msg}`);
            }
        }
    } catch (e) {
        issues.error.push(`Frontmatter validation crashed: ${e.message}`);
    }
    try {
        // validateLinks returns { [relPath]: [brokenLink, ...] }
        const linkErrors = validateLinks(knowledgeDir);
        for (const [file, broken] of Object.entries(linkErrors)) {
            for (const link of broken) {
                issues.error.push(`Broken wikilink in ${file}: ${link}`);
            }
        }
    } catch (e) {
        issues.error.push(`Link validation crashed: ${e.message}`);
    }
    try {
        const orphans = findOrphans(knowledgeDir);
        orphans.forEach(o => issues.warning.push(`Orphan: ${o}`));
    } catch (e) {
        issues.error.push(`Orphan check crashed: ${e.message}`);
    }

    return reportAndExit(issues);
}

function findManifest(knowledgeDir) {
    let dir = path.resolve(knowledgeDir, '..');
    for (let i = 0; i < 4; i++) {
        const candidate = path.join(dir, 'household.json');
        if (fs.existsSync(candidate)) return candidate;
        const parent = path.dirname(dir);
        if (parent === dir) break;
        dir = parent;
    }
    return null;
}

function checkManifest(manifestPath, issues) {
    let manifest;
    try {
        manifest = JSON.parse(fs.readFileSync(manifestPath, 'utf8'));
    } catch (e) {
        issues.error.push(`household.json parse error: ${e.message}`);
        return;
    }
    if (!manifest.meta_repo) {
        issues.error.push('household.json: missing required "meta_repo" field');
        return;
    }
    const workspaceEntry = (manifest.repos || []).find(r => r.name === manifest.meta_repo);
    if (!workspaceEntry) {
        issues.error.push(`household.json: meta_repo = "${manifest.meta_repo}" has no matching repos[] entry`);
    }
    if (manifest.knowledge_base) {
        const kbEntry = (manifest.repos || []).find(r => r.name === manifest.knowledge_base);
        if (!kbEntry) {
            issues.error.push(`household.json: knowledge_base = "${manifest.knowledge_base}" has no matching repos[] entry`);
        }
    }
    const workspaceDir = path.dirname(manifestPath);
    for (const entry of manifest.repos || []) {
        if (entry.name === manifest.meta_repo) continue;
        const dir = path.join(workspaceDir, entry.name);
        if (!fs.existsSync(dir)) {
            issues.warning.push(`Sibling "${entry.name}" declared in household.json but not present at ${dir}`);
        }
    }
}

function checkIndexStaleness(knowledgeDir, issues) {
    const indexPath = path.join(knowledgeDir, '_index.json');
    if (!fs.existsSync(indexPath)) {
        issues.warning.push('_index.json missing — run `make build-index` for fast lookups.');
        return;
    }
    const indexMtime = fs.statSync(indexPath).mtimeMs;
    let staleFiles = 0;
    const walk = (dir) => {
        for (const entry of fs.readdirSync(dir, { withFileTypes: true })) {
            const full = path.join(dir, entry.name);
            if (entry.isDirectory()) walk(full);
            else if (entry.isFile() && entry.name.endsWith('.md')) {
                if (fs.statSync(full).mtimeMs > indexMtime) staleFiles++;
            }
        }
    };
    walk(knowledgeDir);
    if (staleFiles > 0) {
        issues.warning.push(`_index.json is older than ${staleFiles} .md file(s) — run \`make build-index\`.`);
    }
}

function reportAndExit(issues) {
    const lines = [];
    lines.push('Lore + workspace diagnostic\n');
    if (issues.error.length === 0 && issues.warning.length === 0) {
        lines.push('  ✓ All checks passed.');
    } else {
        for (const e of issues.error) lines.push(`  ✗ ${e}`);
        for (const w of issues.warning) lines.push(`  ⚠ ${w}`);
    }
    lines.push('');
    lines.push(`Summary: ${issues.error.length} error(s), ${issues.warning.length} warning(s).`);
    console.log(lines.join('\n'));
    return issues.error.length > 0 ? 1 : 0;
}

module.exports = { runDoctor };
