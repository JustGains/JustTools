import React from 'react';
import {AbsoluteFill, interpolate, useCurrentFrame, useVideoConfig} from 'remotion';
import {c, sans} from './theme';
import {
  Aurora,
  Browser,
  clamp,
  ContextMenu,
  Cursor,
  CursorKey,
  ease,
  Explorer,
  FileItem,
  HeadlessBar,
  Keys,
  ProgressWindow,
  SceneHeader,
  Step,
  Toast,
  useSpring,
} from './ui';

type Rect = {x: number; y: number; w: number; h: number};

// Landscape puts the browser beside Explorer; the 9:16 reel stacks them.
export const useLayout = (): {vertical: boolean; B: Rect; E: Rect} => {
  const {width, height} = useVideoConfig();
  return height > width
    ? {vertical: true, B: {x: 40, y: 330, w: 1000, h: 700}, E: {x: 90, y: 1080, w: 900, h: 600}}
    : {vertical: false, B: {x: 80, y: 196, w: 940, h: 700}, E: {x: 1060, y: 236, w: 780, h: 640}};
};

type Copy = {mode: 'address'} | {mode: 'menu'; x: number; y: number; items: string[]; highlight: number};

export type UseCaseProps = {
  index: string;
  title: string;
  sub: string;
  url: string;
  tab: string;
  tabIcon?: React.ReactNode;
  page: React.ReactNode;
  copy: Copy;
  folder: string;
  files: FileItem[];
  result: FileItem;
  steps: Step[];
  resultAt: number;
  toast: {title: string; body: string};
  cmd: string;
  extra?: React.ReactNode;
  hue?: number;
};

const COPY_CLICK = 40;
const FOCUS_CLICK = 100;
const PASTE = 110;
const PROGRESS = 124;

const CopiedPill: React.FC<{at: number; x: number; y: number}> = ({at, x, y}) => {
  const frame = useCurrentFrame();
  const s = useSpring(at, 14, 0.5);
  if (frame < at || frame > at + 50) return null;
  const out = interpolate(frame, [at + 40, at + 50], [1, 0], clamp);
  return (
    <div
      style={{
        position: 'absolute',
        left: x,
        top: y,
        transform: `translate(-50%, ${(1 - s) * 14}px)`,
        opacity: out * s,
        padding: '8px 16px',
        borderRadius: 999,
        background: c.good,
        color: '#06210f',
        fontFamily: sans,
        fontWeight: 700,
        fontSize: 17,
        boxShadow: '0 10px 30px rgba(111,214,151,0.35)',
      }}
    >
      ✓ Copied
    </div>
  );
};

export const UseCase: React.FC<UseCaseProps> = (p) => {
  const frame = useCurrentFrame();
  const {durationInFrames} = useVideoConfig();
  const {vertical, B, E} = useLayout();
  const inB = useSpring(4, 18);
  const inE = useSpring(12, 18);

  const addressX = B.x + 360;
  const addressY = B.y + 72;
  const target = p.copy.mode === 'address' ? {x: addressX, y: addressY} : {x: B.x + p.copy.x, y: B.y + p.copy.y};
  const fileArea = {x: E.x + 300, y: E.y + 400};

  const cursorKeys: CursorKey[] =
    p.copy.mode === 'address'
      ? [
          {f: 0, x: B.x + 780, y: B.y + 560},
          {f: 14, x: B.x + 780, y: B.y + 560},
          {f: COPY_CLICK, x: target.x, y: target.y, click: true},
          {f: 74, x: target.x + 10, y: target.y + 14},
        ]
      : [
          {f: 0, x: B.x + 780, y: B.y + 620},
          {f: 14, x: B.x + 780, y: B.y + 620},
          {f: COPY_CLICK - 12, x: target.x, y: target.y, click: true},
          {f: COPY_CLICK + 8, x: target.x + 40, y: target.y + 26 + p.copy.highlight * 39},
          {f: COPY_CLICK + 14, x: target.x + 40, y: target.y + 26 + p.copy.highlight * 39, click: true},
          {f: 74, x: target.x + 50, y: target.y + 40 + p.copy.highlight * 39},
        ];
  cursorKeys.push({f: FOCUS_CLICK, x: fileArea.x, y: fileArea.y, click: true});
  cursorKeys.push({f: durationInFrames, x: fileArea.x + 30, y: fileArea.y + 20});

  const selected = p.copy.mode === 'address' ? interpolate(frame, [COPY_CLICK + 1, COPY_CLICK + 5], [0, 1], clamp) : 0;
  const browserFocus = interpolate(frame, [COPY_CLICK - 6, COPY_CLICK, FOCUS_CLICK - 2, FOCUS_CLICK + 2], [0, 1, 1, 0], clamp);
  const explorerFocus = interpolate(frame, [FOCUS_CLICK, FOCUS_CLICK + 6], [0, 1], clamp);

  // Camera: lean toward the browser while copying, then toward Explorer while pasting.
  const pan = interpolate(frame, [0, 50, FOCUS_CLICK - 10, FOCUS_CLICK + 20], [0, 36, 36, -36], {...clamp, easing: ease});
  const zoom = interpolate(frame, [0, durationInFrames], [1, 1.045], clamp);

  const hasProgress = p.steps.length > 0;
  const files = [...p.files, {...p.result, appearAt: p.resultAt}];

  return (
    <AbsoluteFill>
      <Aurora hue={p.hue} />
      <AbsoluteFill style={{transform: `${vertical ? 'translateY' : 'translateX'}(${pan}px) scale(${zoom})`}}>
        <div style={{position: 'absolute', inset: 0, opacity: inB, transform: `translateY(${(1 - inB) * 60}px)`}}>
          <Browser {...B} url={p.url} tab={p.tab} tabIcon={p.tabIcon} selected={selected} focus={browserFocus}>
            {p.page}
          </Browser>
        </div>
        <div style={{position: 'absolute', inset: 0, opacity: inE, transform: `translateY(${(1 - inE) * 60}px)`}}>
          <Explorer {...E} folder={p.folder} files={files} focus={explorerFocus} />
        </div>
        {p.copy.mode === 'menu' ? (
          <ContextMenu
            x={target.x + 6}
            y={target.y + 6}
            at={COPY_CLICK - 11}
            items={p.copy.items}
            highlight={p.copy.highlight}
            highlightAt={COPY_CLICK + 6}
            until={COPY_CLICK + 16}
          />
        ) : null}
        <CopiedPill at={COPY_CLICK + 14} x={target.x} y={target.y + 40} />
        {hasProgress ? (
          <ProgressWindow x={E.x + 330} y={E.y + 420} folder={p.folder} steps={p.steps} start={PROGRESS} end={p.resultAt - 2} />
        ) : null}
        {p.extra}
        <Cursor keys={cursorKeys} />
      </AbsoluteFill>
      {p.copy.mode === 'address' ? <Keys at={COPY_CLICK + 6} keys={['Ctrl', 'C']} x={B.x + B.w / 2} y={B.y + 360} /> : null}
      <Keys at={PASTE} keys={['Ctrl', 'V']} x={E.x + E.w / 2} y={E.y + 300} />
      <Toast at={p.resultAt + 4} title={p.toast.title} body={p.toast.body} y={vertical ? 1700 : 64} x={vertical ? 320 : 1410} />
      <SceneHeader index={p.index} title={p.title} sub={p.sub} />
      <HeadlessBar at={p.resultAt + 16} cmd={p.cmd} />
    </AbsoluteFill>
  );
};
