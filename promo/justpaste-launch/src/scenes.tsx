import React from 'react';
import {AbsoluteFill, interpolate, spring, useCurrentFrame, useVideoConfig} from 'remotion';
import {ArticleIcon, Avatar, PartyCat, SunsetScene, YouTubeIcon} from './pages';
import {brandGradient, c, mono, sans} from './theme';
import {Aurora, clamp, ease, FileGlyph, GitMark, Keys, Logo, SceneHeader, useSpring} from './ui';
import {useLayout} from './UseCase';

/* ---------- Intro ---------- */

const chips: {label: string; icon: React.ReactNode; from: [number, number]}[] = [
  {label: 'youtube.com/watch?v=8fJ2kLmQ0aE', icon: YouTubeIcon, from: [-620, -300]},
  {label: 'party-cat.gif', icon: <div style={{width: 22, height: 22, borderRadius: 5, overflow: 'hidden', position: 'relative'}}><PartyCat small /></div>, from: [640, -320]},
  {label: 'github.com/JustGains/JustTools', icon: <GitMark size={20} />, from: [-700, 40]},
  {label: 'x.com/novapark', icon: <div style={{width: 22, height: 22, position: 'relative'}}><Avatar round /></div>, from: [700, 60]},
  {label: 'edgewise.dev/blog/cold-starts', icon: ArticleIcon, from: [-40, 360]},
];

export const Intro: React.FC = () => {
  const frame = useCurrentFrame();
  const {fps, width, height} = useVideoConfig();
  const sx = width > height ? 1 : 0.3;
  const sy = width > height ? 1 : 2;
  const gather = interpolate(frame, [40, 66], [0, 1], {...clamp, easing: ease});
  const flash = interpolate(frame, [70, 74, 92], [0, 0.9, 0], clamp);
  const logo = spring({frame: frame - 72, fps, config: {damping: 12, mass: 0.8}});
  const title = spring({frame: frame - 82, fps, config: {damping: 16}});
  const tag = spring({frame: frame - 96, fps, config: {damping: 18}});
  const part = spring({frame: frame - 110, fps, config: {damping: 18}});
  return (
    <AbsoluteFill style={{fontFamily: sans, color: c.text}}>
      <Aurora />
      {chips.map((chip, i) => {
        const s = spring({frame: frame - i * 5, fps, config: {damping: 14}});
        const float = Math.sin((frame + i * 20) / 18) * 10;
        const x = chip.from[0] * sx * (1 - gather) * (0.6 + s * 0.4);
        const y = chip.from[1] * sy * (1 - gather) + float * (1 - gather);
        if (frame > 70) return null;
        return (
          <div
            key={chip.label}
            style={{
              position: 'absolute',
              left: '50%',
              top: '50%',
              transform: `translate(-50%,-50%) translate(${x}px, ${y}px) scale(${s * (1 - gather * 0.7)})`,
              opacity: s * (1 - gather * 0.6),
              display: 'flex',
              alignItems: 'center',
              gap: 14,
              padding: '16px 24px',
              borderRadius: 16,
              background: 'rgba(24,26,36,0.9)',
              border: '1px solid rgba(255,255,255,0.12)',
              boxShadow: '0 20px 60px rgba(0,0,0,0.5)',
              fontFamily: mono,
              fontSize: 24,
              whiteSpace: 'nowrap',
            }}
          >
            {chip.icon}
            {chip.label}
          </div>
        );
      })}
      <Keys at={56} keys={['Ctrl', 'V']} x={960} y={540} dur={14} />
      <AbsoluteFill style={{background: `radial-gradient(circle, rgba(160,130,255,${flash}), transparent 60%)`}} />
      <AbsoluteFill style={{alignItems: 'center', justifyContent: 'center', flexDirection: 'column'}}>
        <div style={{transform: `scale(${logo}) rotate(${(1 - logo) * -20}deg)`, opacity: logo}}>
          <Logo size={170} />
        </div>
        <div
          style={{
            fontSize: 150,
            fontWeight: 800,
            letterSpacing: -6,
            marginTop: 18,
            opacity: title,
            transform: `translateY(${(1 - title) * 40}px)`,
            background: 'linear-gradient(90deg,#ffffff 20%,#c9b5ff 60%,#8fb8ff)',
            WebkitBackgroundClip: 'text',
            color: 'transparent',
            lineHeight: 1.05,
          }}
        >
          JustPaste
        </div>
        <div style={{fontSize: 44, fontWeight: 600, color: c.text, textAlign: 'center', maxWidth: width - 120, opacity: tag, transform: `translateY(${(1 - tag) * 20}px)`, marginTop: 8}}>
          Copy anything. <span style={{color: c.violet}}>Ctrl+V</span> it into a folder.
        </div>
        <div style={{fontSize: 24, color: c.dim, marginTop: 26, opacity: part, letterSpacing: 4, textTransform: 'uppercase'}}>Part of JustTools</div>
      </AbsoluteFill>
    </AbsoluteFill>
  );
};

/* ---------- Markdown preview for scene 05 ---------- */

const mdLines: [string, string][] = [
  ['h', '# Shaving cold starts to zero'],
  ['m', ''],
  ['m', '*By Ada Moreno · 7 min read*'],
  ['m', ''],
  ['p', 'Every millisecond before the first byte is a'],
  ['p', 'millisecond your user feels. Here is how we moved'],
  ['p', 'isolates ahead of the request and made **cold'],
  ['p', 'starts disappear**.'],
  ['m', ''],
  ['h', '## Warm before you\'re asked'],
  ['m', ''],
  ['c', '```js'],
  ['c', 'export default {'],
  ['c', '  async fetch(req, env) { // pre-warmed'],
  ['c', '```'],
];

const lineColor = {h: c.violet, p: c.text, m: c.dim, c: c.good};

export const MarkdownPreview: React.FC<{at: number}> = ({at}) => {
  const frame = useCurrentFrame();
  const {B} = useLayout();
  const s = useSpring(at, 18);
  if (frame < at) return null;
  const shown = interpolate(frame, [at + 8, at + 60], [0, mdLines.length], clamp);
  return (
    <div
      style={{
        position: 'absolute',
        left: B.x + 30,
        top: B.y + 40,
        width: B.w - 60,
        height: B.h - 60,
        borderRadius: 14,
        overflow: 'hidden',
        background: '#14151c',
        border: '1px solid rgba(194,145,255,0.35)',
        boxShadow: '0 40px 120px rgba(0,0,0,0.7), 0 0 80px rgba(123,77,255,0.3)',
        opacity: s,
        transform: `translateY(${(1 - s) * 80}px) scale(${0.96 + s * 0.04})`,
        fontFamily: sans,
        color: c.text,
      }}
    >
      <div style={{height: 46, display: 'flex', alignItems: 'center', gap: 10, padding: '0 18px', background: '#1c1d25', borderBottom: `1px solid ${c.line}`, fontSize: 15}}>
        <FileGlyph kind="md" size={22} name="x.md" />
        Shaving cold starts to zero.md
        <div style={{flex: 1}} />
        <span
          style={{
            fontSize: 13,
            padding: '5px 12px',
            borderRadius: 999,
            background: 'rgba(243,128,32,0.15)',
            border: '1px solid rgba(243,128,32,0.5)',
            color: '#ffb36b',
            fontWeight: 600,
          }}
        >
          Rendered by Cloudflare Browser Run · /markdown
        </span>
      </div>
      <div style={{padding: '22px 30px', fontFamily: mono, fontSize: 21, lineHeight: 1.62, whiteSpace: 'pre'}}>
        {mdLines.map(([kind, text], i) => {
          const o = interpolate(shown, [i, i + 1], [0, 1], clamp);
          return (
            <div key={i} style={{display: 'flex', opacity: o, transform: `translateX(${(1 - o) * 12}px)`, minHeight: 34}}>
              <span style={{width: 44, color: '#4a4f5c', fontSize: 16}}>{i + 1}</span>
              <span style={{color: lineColor[kind as keyof typeof lineColor], fontWeight: kind === 'h' ? 700 : 400}}>{text}</span>
            </div>
          );
        })}
      </div>
    </div>
  );
};

/* ---------- Everywhere (terminal) ---------- */

type Line = {t: string; kind: 'cmd' | 'out' | 'ok' | 'dim'};
const term: {at: number; lines: Line[]}[] = [
  {
    at: 10,
    lines: [
      {t: 'justpaste https://www.youtube.com/watch?v=8fJ2kLmQ0aE', kind: 'cmd'},
      {t: 'justpaste: https://www.youtube.com/watch?v=8fJ2kLmQ0aE', kind: 'dim'},
      {t: '  saved   Sunset Drive 4K Lofi Ride Through the Desert.mp4  248.6 MB', kind: 'ok'},
      {t: 'justpaste: 1 saved', kind: 'out'},
    ],
  },
  {
    at: 72,
    lines: [
      {t: 'justpaste https://github.com/JustGains/JustTools -o projects', kind: 'cmd'},
      {t: 'justpaste: https://github.com/JustGains/JustTools', kind: 'dim'},
      {t: '  cloned  projects/JustTools', kind: 'ok'},
      {t: 'justpaste: 1 cloned', kind: 'out'},
    ],
  },
  {
    at: 134,
    lines: [
      {t: 'just paste --enable', kind: 'cmd'},
      {t: 'JustPaste Smart Paste started; settings are in the tray menu.', kind: 'ok'},
    ],
  },
];

const TermLine: React.FC<{line: Line; at: number}> = ({line, at}) => {
  const frame = useCurrentFrame();
  if (frame < at) return null;
  if (line.kind === 'cmd') {
    const n = Math.floor(interpolate(frame, [at, at + 22], [0, line.t.length], clamp));
    const typing = n < line.t.length;
    return (
      <div>
        <span style={{color: c.violet}}>❯ </span>
        <span style={{color: c.text}}>{line.t.slice(0, n)}</span>
        {typing ? <span style={{color: c.accent}}>▍</span> : null}
      </div>
    );
  }
  const color = {out: c.text, ok: c.good, dim: c.muted}[line.kind];
  return <div style={{color}}>{line.t}</div>;
};

export const Everywhere: React.FC = () => {
  const frame = useCurrentFrame();
  const {vertical} = useLayout();
  const T = vertical ? {left: 40, top: 400, width: 1000, height: 560} : {left: 80, top: 220, width: 1120, height: 620};
  const win = useSpring(0, 18);
  const chipsList = ['Windows', 'macOS', 'Linux', 'x64 · ARM64', 'One portable binary'];
  return (
    <AbsoluteFill style={{fontFamily: sans, color: c.text}}>
      <Aurora hue={-20} />
      <SceneHeader index="06" title="Same paste, from any terminal" sub="Links as arguments, piped stdin, or --clipboard. Deterministic and scriptable." />
      <div
        style={{
          position: 'absolute',
          ...T,
          borderRadius: 14,
          background: '#0c0d12',
          border: '1px solid rgba(255,255,255,0.12)',
          boxShadow: '0 40px 120px rgba(0,0,0,0.6)',
          overflow: 'hidden',
          opacity: win,
          transform: `translateY(${(1 - win) * 60}px)`,
        }}
      >
        <div style={{height: 42, display: 'flex', alignItems: 'center', gap: 8, padding: '0 16px', background: '#17181f', fontSize: 14, color: c.dim}}>
          <span style={{width: 12, height: 12, borderRadius: 6, background: '#ff5f57'}} />
          <span style={{width: 12, height: 12, borderRadius: 6, background: '#febc2e'}} />
          <span style={{width: 12, height: 12, borderRadius: 6, background: '#28c840'}} />
          <span style={{marginLeft: 14}}>~/Projects — zsh</span>
        </div>
        <div style={{padding: '22px 26px', fontFamily: mono, fontSize: vertical ? 18 : 21, lineHeight: 1.7, whiteSpace: 'pre'}}>
          {term.map((block) =>
            block.lines.map((line, i) => (
              <TermLine key={block.at + '-' + i} line={line} at={i === 0 ? block.at : block.at + 26 + i * 8} />
            )),
          )}
        </div>
      </div>
      <div
        style={
          vertical
            ? {position: 'absolute', left: 40, top: 1010, width: 1000, display: 'flex', flexWrap: 'wrap', gap: 16}
            : {position: 'absolute', left: 1250, top: 230, width: 590, display: 'flex', flexDirection: 'column', gap: 16}
        }
      >
        {chipsList.map((label, i) => {
          const s = spring({frame: frame - 20 - i * 6, fps: 30, config: {damping: 14}});
          return (
            <div
              key={label}
              style={{
                opacity: s,
                transform: `translateX(${(1 - s) * 60}px)`,
                padding: '18px 24px',
                borderRadius: 14,
                background: 'rgba(24,26,36,0.85)',
                border: '1px solid rgba(255,255,255,0.1)',
                fontSize: 28,
                fontWeight: 600,
                display: 'flex',
                alignItems: 'center',
                gap: 14,
              }}
            >
              <span style={{width: 12, height: 12, borderRadius: 6, background: brandGradient}} />
              {label}
            </div>
          );
        })}
        <div style={{width: '100%'}}>
          <TrayCard at={150} />
        </div>
      </div>
    </AbsoluteFill>
  );
};

const TrayCard: React.FC<{at: number}> = ({at}) => {
  const s = useSpring(at, 15);
  return (
    <div
      style={{
        marginTop: 10,
        opacity: s,
        transform: `scale(${0.9 + s * 0.1})`,
        padding: '20px 24px',
        borderRadius: 16,
        background: 'linear-gradient(135deg, rgba(123,77,255,0.25), rgba(79,125,255,0.2))',
        border: '1px solid rgba(194,145,255,0.45)',
        display: 'flex',
        gap: 18,
        alignItems: 'center',
      }}
    >
      <Logo size={56} />
      <div>
        <div style={{fontSize: 24, fontWeight: 700}}>Smart Paste</div>
        <div style={{fontSize: 18, color: c.dim, marginTop: 4}}>Ctrl+V in any Explorer folder. Lives in your tray.</div>
      </div>
    </div>
  );
};

/* ---------- Outro ---------- */

export const Outro: React.FC = () => {
  const frame = useCurrentFrame();
  const {fps} = useVideoConfig();
  const logo = spring({frame, fps, config: {damping: 13}});
  const t = spring({frame: frame - 8, fps, config: {damping: 16}});
  const cmd = spring({frame: frame - 24, fps, config: {damping: 16}});
  const foot = spring({frame: frame - 40, fps, config: {damping: 18}});
  const thumbs = [<SunsetScene small key="v" />, <PartyCat small key="g" />, null, <Avatar key="a" />, null];
  return (
    <AbsoluteFill style={{fontFamily: sans, color: c.text}}>
      <Aurora hue={10} />
      <AbsoluteFill style={{alignItems: 'center', justifyContent: 'center', flexDirection: 'column'}}>
        <div style={{display: 'flex', gap: 22, marginBottom: 50}}>
          {thumbs.map((th, i) => {
            const s = spring({frame: frame - 4 - i * 4, fps, config: {damping: 12}});
            const kinds = ['video', 'gif', 'repo', 'image', 'md'] as const;
            return (
              <div
                key={i}
                style={{
                  width: 120,
                  height: 120,
                  borderRadius: 20,
                  background: 'rgba(24,26,36,0.9)',
                  border: '1px solid rgba(255,255,255,0.12)',
                  display: 'grid',
                  placeItems: 'center',
                  overflow: 'hidden',
                  position: 'relative',
                  transform: `translateY(${(1 - s) * 40 + Math.sin((frame + i * 12) / 16) * 6}px) scale(${s})`,
                }}
              >
                {th ?? <FileGlyph kind={kinds[i]} size={70} />}
              </div>
            );
          })}
        </div>
        <div style={{display: 'flex', alignItems: 'center', gap: 30, transform: `scale(${0.8 + logo * 0.2})`, opacity: logo}}>
          <Logo size={130} />
          <div style={{fontSize: 130, fontWeight: 800, letterSpacing: -5, opacity: t}}>JustPaste</div>
        </div>
        <div style={{fontSize: 38, color: c.dim, marginTop: 10, opacity: t}}>Copy anything. Ctrl+V it into a folder.</div>
        <div
          style={{
            marginTop: 50,
            opacity: cmd,
            transform: `translateY(${(1 - cmd) * 20}px)`,
            fontFamily: mono,
            fontSize: 34,
            padding: '20px 36px',
            borderRadius: 16,
            background: '#0c0d12',
            border: '1px solid rgba(194,145,255,0.45)',
            boxShadow: '0 0 60px rgba(123,77,255,0.35)',
          }}
        >
          <span style={{color: c.violet}}>❯ </span>just paste --enable
        </div>
        <div style={{marginTop: 46, fontSize: 24, lineHeight: 1.6, color: c.dim, opacity: foot, letterSpacing: 1, textAlign: 'center', maxWidth: 900}}>
          Part of <b style={{color: c.text}}>JustTools</b> · Windows · macOS · Linux · github.com/JustGains/JustTools
        </div>
      </AbsoluteFill>
    </AbsoluteFill>
  );
};
