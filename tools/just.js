#!/usr/bin/env node
// just — pick one of the just* tools from a two-column menu and run it.
// Left column: tools. Right column: what the highlighted tool does, parsed
// live from each script's header comment (so it always matches -h). Enter
// runs the tool in the current directory; its own prompts take over.
//
// usage: just                 open the selector
//        just <tool> [args]   run a tool directly ("just webp -f" = justwebp -f)
//        just -h              list the tools without the menu
//
// UI adapted from F:/bro-cli src/ui.js (zero-dependency readline selector).

import fs from 'node:fs';
import path from 'node:path';
import readline from 'node:readline';
import { spawn } from 'node:child_process';
import { fileURLToPath } from 'node:url';

const BIN = path.dirname(fileURLToPath(import.meta.url));
const stdin = process.stdin;
const stdout = process.stdout;
const isInteractive = Boolean(stdin.isTTY && stdout.isTTY);

// Tools whose extensionless file is only a shim (no doc header) get docs here.
const OVERRIDES = {
  justzip: {
    desc: 'zip a folder into <name>.zip, skipping everything .gitignore ignores',
    help: [
      'justzip — zip a folder into <name>.zip, skipping ignored files.',
      '',
      'usage: justzip [folder]',
      '  with no folder: zips the current folder',
      '  the file list comes from git ls-files, so nested .gitignore files,',
      '  .git/info/exclude and global ignores all apply (the folder must be',
      '  inside a git repository); the .zip lands in the current directory',
      '  at maximum compression'
    ]
  }
};

// Leading comment block of a bash script, shebang skipped, "# " stripped.
function parseHeader(file) {
  let text;
  try {
    text = fs.readFileSync(file, 'utf8');
  } catch {
    return [];
  }
  const lines = text.split(/\r?\n/);
  let i = lines[0]?.startsWith('#!') ? 1 : 0;
  const block = [];
  for (; i < lines.length && lines[i].startsWith('#'); i++) {
    block.push(lines[i].replace(/^# ?/, ''));
  }
  return block;
}

function discoverTools() {
  return fs
    .readdirSync(BIN)
    .filter((f) => /^just[a-z0-9]+$/.test(f) && f !== 'just')
    .sort()
    .map((name) => {
      const o = OVERRIDES[name];
      const help = o?.help ?? parseHeader(path.join(BIN, name));
      const first = help[0] || '';
      const desc = (o?.desc ?? (first.includes('— ') ? first.split('— ')[1] : first)).replace(/[.,;]$/, '');
      return { name, desc, help };
    })
    .filter((t) => t.help.length);
}

function listTools(tools) {
  const w = Math.max(...tools.map((t) => t.name.length));
  stdout.write('\x1b[1mjust\x1b[0m — run one of the just* tools\n\n');
  for (const t of tools) stdout.write(`  \x1b[1m${t.name.padEnd(w)}\x1b[0m  \x1b[2m${t.desc}\x1b[0m\n`);
  stdout.write('\nrun: just <tool> [args]   (e.g. `just webp -f`, `just -h`)\n');
}

function run(name, args) {
  const cmdShim = path.join(BIN, `${name}.cmd`);
  const child =
    process.platform === 'win32' && fs.existsSync(cmdShim)
      ? spawn('cmd.exe', ['/c', cmdShim, ...args], { stdio: 'inherit' })
      : spawn(path.join(BIN, name), args, { stdio: 'inherit' });
  child.on('error', (err) => {
    console.error(`just: failed to run ${name}: ${err.message}`);
    process.exit(1);
  });
  child.on('exit', (code, signal) => process.exit(signal ? 1 : (code ?? 1)));
}

// Word-wrap plain text to `width`, continuation lines inheriting the indent.
function wrapLines(lines, width) {
  const out = [];
  for (const raw of lines) {
    if (!raw) {
      out.push('');
      continue;
    }
    const indent = (raw.match(/^ */)?.[0] ?? '') + '  ';
    let line = raw;
    while ([...line].length > width) {
      let cut = line.lastIndexOf(' ', width);
      if (cut <= indent.length) cut = width;
      out.push(line.slice(0, cut));
      line = indent + line.slice(cut).trimStart();
    }
    out.push(line);
  }
  return out;
}

// Pad or truncate plain (ANSI-free) text to exactly `width` columns.
function fitPlain(s, width) {
  const chars = [...s];
  if (chars.length > width) return chars.slice(0, Math.max(0, width - 1)).join('') + '…';
  return s + ' '.repeat(width - chars.length);
}

// If the process dies while the menu owns the screen, put the terminal back.
let restoreSeq = null;
process.on('exit', () => {
  if (!restoreSeq) return;
  stdout.write(restoreSeq);
  try {
    stdin.setRawMode(false);
  } catch {}
});

// Two-column selector in the bro-cli style: cursor-up overdraw repaints, no
// autowrap while painting, hidden cursor, coalesced resize, inverse-video
// selection. Resolves the chosen tool; rejects Error('cancelled') on esc.
function selectTool(tools) {
  return new Promise((resolve, reject) => {
    let index = 0;
    let paintedLines = 0;
    let visible, lines, leftW, rightW, wrapped;

    const layout = () => {
      const cols = stdout.columns || 80;
      leftW = Math.min(Math.max(...tools.map((t) => t.name.length), 10) + 4, Math.floor((cols - 3) / 2));
      rightW = Math.max(20, cols - leftW - 3);
      wrapped = tools.map((t) => wrapLines(t.help, rightW - 2));
      const tallest = Math.max(tools.length, ...wrapped.map((w) => w.length));
      visible = Math.min(tallest, Math.max(4, (stdout.rows || 30) - 4));
      lines = visible + 1; // message row + choice rows; hint row carries the cursor
    };
    layout();

    readline.emitKeypressEvents(stdin);
    stdin.setRawMode(true);
    stdin.resume();
    restoreSeq = '\x1b[?25h\x1b[?7h';
    stdout.write('\x1b[?25l');

    const paint = (mode) => {
      let out = '\x1b[?7l';
      if (mode === 'repaint') out += `\r\x1b[${paintedLines}A`;
      else if (mode === 'fresh') out += '\x1b[2J\x1b[H';
      out += '\x1b[0J';
      out += `\x1b[1mjust\x1b[0m \x1b[2m·\x1b[0m run which tool?\n`;
      const help = wrapped[index];
      for (let r = 0; r < visible; r++) {
        const t = tools[r];
        const left =
          t === undefined ? ' '.repeat(leftW)
          : r === index ? `\x1b[7m${fitPlain(' ❯ ' + t.name, leftW)}\x1b[0m`
          : fitPlain('   ' + t.name, leftW);
        let right = '';
        const truncated = help.length > visible && r === visible - 1;
        const line = truncated ? '…' : (help[r] ?? '');
        if (line) {
          const text = fitPlain('  ' + line, rightW).trimEnd();
          right = r === 0 ? `\x1b[1m${text}\x1b[0m` : `\x1b[2m${text}\x1b[0m`;
        }
        out += `${left} \x1b[2m│\x1b[0m ${right}\n`;
      }
      out += `\x1b[2m  ↑/↓ move · enter run · esc cancel\x1b[0m`;
      out += '\x1b[?7h';
      stdout.write(out);
      paintedLines = lines;
    };

    let resizeTimer = null;
    const onResize = () => {
      if (resizeTimer) clearTimeout(resizeTimer);
      resizeTimer = setTimeout(() => {
        resizeTimer = null;
        layout();
        paint('fresh');
      }, 80);
    };
    stdout.on('resize', onResize);

    const cleanup = () => {
      if (resizeTimer) clearTimeout(resizeTimer);
      stdin.removeListener('keypress', onKey);
      stdout.removeListener('resize', onResize);
      stdin.setRawMode(false);
      stdin.pause();
      restoreSeq = null;
      stdout.write('\x1b[?25h\x1b[?7h\n');
    };

    const onKey = (str, key) => {
      if (!key) return;
      if (key.name === 'up' || key.name === 'k') {
        index = (index - 1 + tools.length) % tools.length;
        paint('repaint');
      } else if (key.name === 'down' || key.name === 'j') {
        index = (index + 1) % tools.length;
        paint('repaint');
      } else if (key.name === 'home') {
        index = 0;
        paint('repaint');
      } else if (key.name === 'end') {
        index = tools.length - 1;
        paint('repaint');
      } else if (key.name === 'return' || key.name === 'enter') {
        cleanup();
        resolve(tools[index]);
      } else if (key.name === 'escape' || (key.ctrl && key.name === 'c')) {
        cleanup();
        reject(new Error('cancelled'));
      }
    };

    stdin.on('keypress', onKey);
    paint('first');
  });
}

const tools = discoverTools();
if (!tools.length) {
  console.error('just: no just* tools found beside this script');
  process.exit(1);
}

const argv = process.argv.slice(2);
if (argv[0] === '-h' || argv[0] === '--help') {
  listTools(tools);
  process.exit(0);
}

if (argv.length && !argv[0].startsWith('-')) {
  // Direct mode: "just webp -f" runs justwebp -f.
  const want = argv[0].startsWith('just') ? argv[0] : `just${argv[0]}`;
  const tool = tools.find((t) => t.name === want);
  if (!tool) {
    console.error(`just: unknown tool: ${argv[0]}\n`);
    listTools(tools);
    process.exit(2);
  }
  run(tool.name, argv.slice(1));
} else if (!isInteractive) {
  listTools(tools);
  process.exit(0);
} else {
  try {
    const tool = await selectTool(tools);
    stdout.write(`\x1b[2m» ${tool.name}\x1b[0m\n`);
    run(tool.name, []);
  } catch {
    stdout.write('\x1b[2mcancelled\x1b[0m\n');
    process.exit(130);
  }
}
