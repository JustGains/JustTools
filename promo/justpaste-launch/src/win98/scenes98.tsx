import React from 'react';
import {interpolate, spring, useCurrentFrame, useVideoConfig} from 'remotion';
import {linearTiming, TransitionSeries} from '@remotion/transitions';
import {fade} from '@remotion/transitions/fade';
import {Avatar, PartyCat, SunsetScene} from '../pages';
import {EVERYWHERE, INTRO, OUTRO, scenes, T} from '../sceneData';
import {clamp} from '../ui';
import {
  Button98,
  ComputerIcon,
  Desktop,
  DocIcon,
  dos,
  DosIcon,
  FileIcon98,
  FolderIcon,
  GlobeIcon,
  Header98,
  k,
  Keys98,
  Logo98,
  MenuBar,
  NotepadIcon,
  raised,
  Stage,
  sunken,
  Taskbar,
  useStage,
  Window,
} from './ui98';
import {UseCase98, useLayout98} from './UseCase98';

/* ---------- Sky ---------- */

const Clouds: React.FC = () => {
  const frame = useCurrentFrame();
  const {w, h} = useStage();
  const clouds = [
    [0.1, 0.12, 1.2],
    [0.62, 0.08, 1],
    [0.32, 0.38, 1.4],
    [0.82, 0.46, 1.1],
    [0.05, 0.7, 1.3],
    [0.55, 0.78, 1.2],
    [0.3, 0.92, 1],
  ];
  return (
    <div style={{position: 'absolute', inset: 0, background: 'linear-gradient(180deg,#2f63c4 0%,#6fa6ea 55%,#bcd9f6 100%)', overflow: 'hidden'}}>
      {clouds.map(([cx, cy, s], i) => (
        <div
          key={i}
          style={{
            position: 'absolute',
            left: cx * w + ((frame * (0.15 + i * 0.04)) % (w * 0.3)) - 60,
            top: cy * h - 30,
            width: 180 * s,
            height: 60 * s,
            borderRadius: '50%',
            background: 'radial-gradient(ellipse, rgba(255,255,255,0.95) 0%, rgba(255,255,255,0.6) 45%, transparent 70%)',
            filter: 'blur(2px)',
          }}
        />
      ))}
    </div>
  );
};

const BootBar: React.FC = () => {
  const frame = useCurrentFrame();
  return (
    <div
      style={{
        position: 'absolute',
        left: 0,
        right: 0,
        bottom: 0,
        height: 7,
        background: 'linear-gradient(90deg,#0b2a8c,#3f8cff,#a6e0ff,#3f8cff,#0b2a8c)',
        backgroundSize: '200% 100%',
        backgroundPosition: `${-frame * 4}px 0`,
      }}
    />
  );
};

const Wordmark: React.FC<{size: number}> = ({size}) => (
  <div
    style={{
      fontFamily: '"Franklin Gothic Medium", "Arial Black", Arial, sans-serif',
      fontStyle: 'italic',
      fontSize: size,
      fontWeight: 700,
      color: '#fff',
      letterSpacing: -1,
      textShadow: '2px 2px 0 #10247a, 4px 4px 0 rgba(0,0,0,0.25)',
      lineHeight: 1,
    }}
  >
    JustPaste
  </div>
);

/* ---------- Intro ---------- */

const chips: {label: string; icon: React.ReactNode; from: [number, number]}[] = [
  {label: 'youtube.com/watch…', icon: <GlobeIcon />, from: [-0.32, -0.3]},
  {label: 'party-cat.gif', icon: <FileIcon98 kind="gif" name="party-cat.gif" />, from: [0.32, -0.3]},
  {label: 'JustGains/JustTools', icon: <FolderIcon git />, from: [-0.36, 0.06]},
  {label: 'x.com/novapark', icon: <div style={{width: 32, height: 32, position: 'relative', border: '1px solid #000'}}><Avatar /></div>, from: [0.36, 0.06]},
  {label: 'cold-starts.md', icon: <NotepadIcon />, from: [0, 0.32]},
];

export const Intro98: React.FC = () => {
  const frame = useCurrentFrame();
  const {fps} = useVideoConfig();
  const {w, h} = useStage();
  const gather = interpolate(frame, [40, 66], [0, 1], clamp);
  const flash = interpolate(frame, [70, 73, 90], [0, 1, 0], clamp);
  const logo = spring({frame: frame - 72, fps, config: {damping: 13}});
  const tag = interpolate(frame, [92, 100], [0, 1], clamp);
  const part = interpolate(frame, [104, 112], [0, 1], clamp);
  return (
    <Stage>
      <Clouds />
      {frame <= 70
        ? chips.map((chip, i) => {
            const s = interpolate(frame, [i * 5, i * 5 + 6], [0, 1], clamp);
            const x = w / 2 + chip.from[0] * w * (1 - gather);
            const y = h / 2 + chip.from[1] * h * (1 - gather);
            return (
              <div
                key={chip.label}
                style={{
                  position: 'absolute',
                  left: x,
                  top: y,
                  transform: `translate(-50%,-50%) scale(${s * (1 - gather * 0.6)})`,
                  display: 'flex',
                  flexDirection: 'column',
                  alignItems: 'center',
                  gap: 3,
                  opacity: 1 - gather * 0.5,
                }}
              >
                {chip.icon}
                <span style={{background: k.navy, color: '#fff', padding: '0 3px', outline: '1px dotted #ffff80', fontSize: 12, whiteSpace: 'nowrap'}}>{chip.label}</span>
              </div>
            );
          })
        : null}
      <Keys98 at={54} keys={['Ctrl', 'V']} x={w / 2} y={h / 2} dur={14} />
      <div style={{position: 'absolute', inset: 0, background: '#fff', opacity: flash}} />
      {frame >= 72 ? (
        <div style={{position: 'absolute', inset: 0, display: 'flex', flexDirection: 'column', alignItems: 'center', justifyContent: 'center', gap: 10}}>
          <div style={{display: 'flex', alignItems: 'center', gap: 14, transform: `scale(${logo})`}}>
            <Logo98 size={72} />
            <Wordmark size={76} />
          </div>
          <div style={{color: '#fff', fontSize: 20, fontWeight: 700, textShadow: '1px 1px 0 #10247a', opacity: tag, textAlign: 'center', padding: '0 20px'}}>
            Copy anything. Ctrl+V it into a folder.
          </div>
          <div style={{color: '#e8f2ff', fontSize: 12, letterSpacing: 3, opacity: part, textShadow: '1px 1px 0 #10247a'}}>PART OF JUSTTOOLS</div>
        </div>
      ) : null}
      <BootBar />
    </Stage>
  );
};

/* ---------- Notepad (scene 05) ---------- */

const mdLines: [string, string][] = [
  ['h', '# Shaving cold starts to zero'],
  ['m', ''],
  ['m', '*By Ada Moreno · 7 min read*'],
  ['m', ''],
  ['p', 'Every millisecond before the first byte is a'],
  ['p', 'millisecond your user feels. Here is how we'],
  ['p', 'moved isolates ahead of the request and made'],
  ['p', '**cold starts disappear**.'],
  ['m', ''],
  ['h', "## Warm before you're asked"],
  ['m', ''],
  ['c', '```js'],
  ['c', 'export default {'],
  ['c', '  async fetch(req, env) { // pre-warmed'],
  ['c', '```'],
];

export const Notepad98: React.FC<{at: number}> = ({at}) => {
  const frame = useCurrentFrame();
  const {B} = useLayout98();
  if (frame < at) return null;
  const shown = interpolate(frame, [at + 6, at + 56], [0, mdLines.length], clamp);
  return (
    <Window x={B.x + 14} y={B.y + 22} w={B.w - 28} h={B.h - 40} title="Shaving cold starts to zero.md - Notepad" icon={<NotepadIcon size={14} />} style={{zIndex: 6}}>
      <MenuBar items={['File', 'Edit', 'Search', 'Help']} />
      <div style={{flex: 1, background: '#fff', boxShadow: sunken, padding: '6px 8px', position: 'relative', overflow: 'hidden'}}>
        <div style={{position: 'absolute', right: 6, top: 4, background: k.tip, border: '1px solid #000', padding: '2px 5px', fontSize: 10, boxShadow: '1px 1px 0 rgba(0,0,0,0.3)'}}>
          Rendered by Cloudflare Browser Run · /markdown
        </div>
        <div style={{fontFamily: dos, fontSize: 11, lineHeight: '16px', whiteSpace: 'pre', marginTop: 24}}>
          {mdLines.map(([, text], i) => (
            <div key={i} style={{opacity: shown > i ? 1 : 0, minHeight: 16}}>
              {text}
              {Math.floor(shown) === i ? <span style={{background: '#000', color: '#000'}}>|</span> : null}
            </div>
          ))}
        </div>
      </div>
    </Window>
  );
};

/* ---------- Everywhere (MS-DOS Prompt + properties dialog) ---------- */

type Line = {t: string; cmd?: boolean};
const term: {at: number; lines: Line[]}[] = [
  {
    at: 12,
    lines: [
      {t: 'justpaste https://www.youtube.com/watch?v=8fJ2kLmQ0aE', cmd: true},
      {t: 'justpaste: https://www.youtube.com/watch?v=8fJ2kLmQ0aE'},
      {t: '  saved   Sunset Drive 4K Lofi Ride Through the Desert.mp4  248.6 MB'},
      {t: 'justpaste: 1 saved'},
    ],
  },
  {
    at: 74,
    lines: [
      {t: 'justpaste https://github.com/JustGains/JustTools -o projects', cmd: true},
      {t: 'justpaste: https://github.com/JustGains/JustTools'},
      {t: '  cloned  projects\\JustTools'},
      {t: 'justpaste: 1 cloned'},
    ],
  },
  {
    at: 136,
    lines: [
      {t: 'just paste --enable', cmd: true},
      {t: 'JustPaste Smart Paste started; settings are in the tray menu.'},
    ],
  },
];

const PROMPT = 'C:\\PROJECTS>';

export const Everywhere98: React.FC = () => {
  const frame = useCurrentFrame();
  const {vertical} = useStage();
  const D = vertical ? {x: 10, y: 140, w: 520, h: 360} : {x: 84, y: 66, w: 548, h: 370};
  const P = vertical ? {x: 70, y: 520, w: 400, h: 330} : {x: 646, y: 86, w: 300, h: 350};
  const checks = ['Windows', 'macOS', 'Linux', 'x64 and ARM64', 'One portable binary'];
  const dialogIn = interpolate(frame, [18, 24], [0, 1], clamp);
  return (
    <Stage>
      <Desktop />
      <Header98 index="06" title="Same paste, from any terminal" sub="Links as arguments, piped stdin, or --clipboard. Deterministic and scriptable." />
      <Window {...D} title="MS-DOS Prompt" icon={<DosIcon size={14} />} active={frame < 150}>
        <div style={{height: 22, display: 'flex', alignItems: 'center', gap: 4, padding: '0 4px'}}>
          <div style={{width: 60, height: 18, background: '#fff', boxShadow: sunken, padding: '0 3px', display: 'flex', alignItems: 'center'}}>Auto ▾</div>
        </div>
        <div style={{flex: 1, background: '#000', color: '#c0c0c0', fontFamily: dos, fontSize: 9.6, lineHeight: '16px', padding: 6, whiteSpace: 'pre', overflow: 'hidden', boxShadow: sunken}}>
          {term.map((block) =>
            block.lines.map((line, i) => {
              const at = i === 0 ? block.at : block.at + 26 + i * 8;
              if (frame < at) return null;
              if (line.cmd) {
                const n = Math.floor(interpolate(frame, [at, at + 22], [0, line.t.length], clamp));
                return (
                  <div key={block.at + '-' + i}>
                    {PROMPT}
                    {line.t.slice(0, n)}
                    {n < line.t.length ? '_' : ''}
                  </div>
                );
              }
              return (
                <div key={block.at + '-' + i} style={{color: line.t.startsWith('  ') || line.t.startsWith('JustPaste') ? '#fff' : '#c0c0c0'}}>
                  {line.t}
                </div>
              );
            }),
          )}
          {frame > 170 ? (
            <div>
              {PROMPT}
              <span style={{opacity: Math.floor(frame / 8) % 2}}>_</span>
            </div>
          ) : null}
        </div>
      </Window>
      <div style={{opacity: dialogIn}}>
        <Window {...P} title="JustPaste Properties" icon={<Logo98 size={13} />} help active={frame >= 150}>
          <div style={{display: 'flex', gap: 0, padding: '6px 6px 0'}}>
            {['General', 'Smart Paste', 'Filters'].map((tab, i) => (
              <div
                key={tab}
                style={{
                  padding: '2px 8px',
                  boxShadow: `inset 1px 1px ${k.white}, inset -1px 0 ${k.dark}`,
                  background: k.face,
                  marginBottom: i === 0 ? -2 : 0,
                  position: 'relative',
                  zIndex: i === 0 ? 2 : 1,
                  paddingBottom: i === 0 ? 4 : 2,
                }}
              >
                {tab}
              </div>
            ))}
          </div>
          <div style={{flex: 1, margin: '0 6px', boxShadow: raised, padding: 10, display: 'flex', flexDirection: 'column', gap: 8}}>
            <div style={{display: 'flex', gap: 10, alignItems: 'center'}}>
              <Logo98 size={32} />
              <div>
                <b style={{fontSize: 13}}>JustPaste</b>
                <div>Part of JustTools</div>
              </div>
            </div>
            <fieldset style={{border: 'none', boxShadow: `inset 1px 1px ${k.shadow}, inset -1px -1px ${k.white}, 1px 1px ${k.white}`, padding: '10px 8px 6px', margin: 0}}>
              <legend style={{padding: '0 3px'}}>Runs on</legend>
              {checks.map((label, i) => {
                const on = frame >= 30 + i * 8;
                return (
                  <div key={label} style={{display: 'flex', alignItems: 'center', gap: 6, height: 17}}>
                    <div style={{width: 13, height: 13, background: '#fff', boxShadow: sunken, display: 'grid', placeItems: 'center', fontSize: 10, fontWeight: 700}}>{on ? '✓' : ''}</div>
                    {label}
                  </div>
                );
              })}
            </fieldset>
            <div style={{display: 'flex', gap: 8, alignItems: 'center', opacity: interpolate(frame, [150, 158], [0, 1], clamp)}}>
              <ComputerIcon />
              <div style={{lineHeight: '14px'}}>
                <b>Smart Paste is on.</b>
                <br />
                Ctrl+V in any Explorer folder. Lives in your tray.
              </div>
            </div>
          </div>
          <div style={{display: 'flex', justifyContent: 'flex-end', gap: 6, padding: 6}}>
            <Button98 down={frame >= 180 && frame < 186}>OK</Button98>
            <Button98>Cancel</Button98>
          </div>
        </Window>
      </div>
      <Taskbar
        trayBlink={frame >= 136 && frame < 150}
        tasks={[
          {label: 'MS-DOS Prompt', icon: <DosIcon size={14} />, active: frame < 150},
          {label: 'JustPaste Properties', icon: <Logo98 size={14} />, active: frame >= 150},
        ]}
      />
    </Stage>
  );
};

/* ---------- Outro ---------- */

export const Outro98: React.FC = () => {
  const frame = useCurrentFrame();
  const {fps} = useVideoConfig();
  const {vertical} = useStage();
  const logo = spring({frame: frame - 4, fps, config: {damping: 13}});
  const cmd = interpolate(frame, [26, 32], [0, 1], clamp);
  const foot = interpolate(frame, [40, 48], [0, 1], clamp);
  const items: React.ReactNode[] = [
    <SunsetScene small key="v" />,
    <PartyCat small key="g" />,
    <div key="r" style={{display: 'grid', placeItems: 'center', height: '100%', background: '#fff'}}><FolderIcon git /></div>,
    <Avatar key="a" />,
    <div key="m" style={{display: 'grid', placeItems: 'center', height: '100%', background: '#fff'}}><DocIcon label="MD" color="#600080" /></div>,
  ];
  return (
    <Stage>
      <Clouds />
      <div style={{position: 'absolute', inset: 0, display: 'flex', flexDirection: 'column', alignItems: 'center', justifyContent: 'center', gap: 14}}>
        <div style={{display: 'flex', gap: 10}}>
          {items.map((it, i) => {
            const s = interpolate(frame, [i * 3, i * 3 + 6], [0, 1], clamp);
            return (
              <div key={i} style={{width: 56, height: 56, padding: 3, background: k.face, boxShadow: raised, transform: `scale(${s})`}}>
                <div style={{position: 'relative', width: '100%', height: '100%', overflow: 'hidden', boxShadow: sunken}}>{it}</div>
              </div>
            );
          })}
        </div>
        <div style={{display: 'flex', alignItems: 'center', gap: 12, transform: `scale(${logo})`, marginTop: 10}}>
          <Logo98 size={vertical ? 60 : 66} />
          <Wordmark size={vertical ? 64 : 70} />
        </div>
        <div style={{color: '#fff', fontSize: 18, fontWeight: 700, textShadow: '1px 1px 0 #10247a', textAlign: 'center', padding: '0 20px'}}>Copy anything. Ctrl+V it into a folder.</div>
        <div style={{opacity: cmd, marginTop: 6}}>
          <div style={{width: vertical ? 320 : 340, background: k.face, boxShadow: raised, padding: 3}}>
            <div style={{height: 16, background: k.title, color: '#fff', fontWeight: 700, padding: '0 4px', display: 'flex', alignItems: 'center', gap: 4}}>
              <DosIcon size={12} /> MS-DOS Prompt
            </div>
            <div style={{background: '#000', color: '#fff', fontFamily: dos, fontSize: 16, padding: '10px 10px'}}>
              C:\&gt;just paste --enable<span style={{opacity: Math.floor(frame / 8) % 2}}>_</span>
            </div>
          </div>
        </div>
        <div style={{opacity: foot, color: '#fff', textShadow: '1px 1px 0 #10247a', fontSize: 12, textAlign: 'center', lineHeight: 1.5, padding: '0 20px', marginTop: 4}}>
          Part of <b>JustTools</b> · Windows · macOS · Linux
          <br />
          github.com/JustGains/JustTools
        </div>
      </div>
      <BootBar />
    </Stage>
  );
};

/* ---------- Full sequence ---------- */

export const Launch98: React.FC = () => (
  <TransitionSeries>
    <TransitionSeries.Sequence durationInFrames={INTRO}>
      <Intro98 />
    </TransitionSeries.Sequence>
    {scenes.map(({duration, ...props}) => (
      <React.Fragment key={props.index}>
        <TransitionSeries.Transition presentation={fade()} timing={linearTiming({durationInFrames: T})} />
        <TransitionSeries.Sequence durationInFrames={duration}>
          <UseCase98 {...props} extra98={props.index === '05' ? <Notepad98 at={210} /> : undefined} />
        </TransitionSeries.Sequence>
      </React.Fragment>
    ))}
    <TransitionSeries.Transition presentation={fade()} timing={linearTiming({durationInFrames: T})} />
    <TransitionSeries.Sequence durationInFrames={EVERYWHERE}>
      <Everywhere98 />
    </TransitionSeries.Sequence>
    <TransitionSeries.Transition presentation={fade()} timing={linearTiming({durationInFrames: T})} />
    <TransitionSeries.Sequence durationInFrames={OUTRO}>
      <Outro98 />
    </TransitionSeries.Sequence>
  </TransitionSeries>
);
