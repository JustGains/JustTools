import React from 'react';
import {interpolate, useCurrentFrame, useVideoConfig} from 'remotion';
import {clamp} from '../ui';
import {UseCaseProps} from '../UseCase';
import {
  Balloon,
  Browser98,
  Cursor98,
  Desktop,
  DosBar,
  Explorer98,
  FolderIcon,
  GlobeIcon,
  Header98,
  Key98,
  Keys98,
  Menu98,
  MENU_ROW,
  ProgressDialog98,
  Rect,
  Stage,
  Taskbar,
  useStage,
} from './ui98';

const COPY_CLICK = 40;
const FOCUS_CLICK = 100;
const PASTE = 110;
const PROGRESS = 124;
const CONTENT_TOP = 100;

export const useLayout98 = (): {B: Rect; E: Rect; D: Rect; pane: number} => {
  const {vertical} = useStage();
  return vertical
    ? {B: {x: 10, y: 132, w: 520, h: 380}, E: {x: 24, y: 526, w: 492, h: 290}, D: {x: 10, y: 824, w: 520, h: 46}, pane: 150}
    : {B: {x: 84, y: 62, w: 450, h: 370}, E: {x: 546, y: 80, w: 404, h: 350}, D: {x: 84, y: 446, w: 520, h: 50}, pane: 132};
};

export const UseCase98: React.FC<UseCaseProps & {extra98?: React.ReactNode}> = (p) => {
  const frame = useCurrentFrame();
  const {durationInFrames} = useVideoConfig();
  const {B, E, D, pane} = useLayout98();

  const target =
    p.copy.mode === 'address'
      ? {x: B.x + 140, y: B.y + 87}
      : {x: B.x + 5 + p.copy.x / 2, y: B.y + CONTENT_TOP + (p.copy.y - 98) / 2};
  const fileArea = {x: E.x + pane + 60, y: E.y + E.h - 62};
  const menuRowY = (i: number) => target.y + 4 + 3 + i * MENU_ROW + 8;

  const keys: Key98[] =
    p.copy.mode === 'address'
      ? [
          {f: 0, x: B.x + B.w - 60, y: B.y + B.h - 80},
          {f: 14, x: B.x + B.w - 60, y: B.y + B.h - 80},
          {f: COPY_CLICK, x: target.x, y: target.y, click: true},
          {f: 74, x: target.x + 6, y: target.y + 8},
        ]
      : [
          {f: 0, x: B.x + B.w - 60, y: B.y + B.h - 60},
          {f: 14, x: B.x + B.w - 60, y: B.y + B.h - 60},
          {f: COPY_CLICK - 12, x: target.x, y: target.y, click: true},
          {f: COPY_CLICK + 8, x: target.x + 30, y: menuRowY(p.copy.highlight)},
          {f: COPY_CLICK + 14, x: target.x + 30, y: menuRowY(p.copy.highlight), click: true},
          {f: 74, x: target.x + 34, y: menuRowY(p.copy.highlight) + 10},
        ];
  keys.push({f: FOCUS_CLICK, x: fileArea.x, y: fileArea.y, click: true});
  keys.push({f: durationInFrames, x: fileArea.x + 10, y: fileArea.y + 6});

  const selected = p.copy.mode === 'address' ? interpolate(frame, [COPY_CLICK + 1, COPY_CLICK + 3], [0, 1], clamp) : 0;
  const explorerActive = frame >= FOCUS_CLICK;
  const files = [...p.files, {...p.result, appearAt: p.resultAt}];
  const browserIn = interpolate(frame, [2, 8], [0, 1], clamp);
  const explorerIn = interpolate(frame, [8, 14], [0, 1], clamp);

  return (
    <Stage>
      <Desktop />
      <Header98 index={p.index} title={p.title} sub={p.sub} />
      <div style={{opacity: browserIn}}>
        <Browser98 {...B} url={p.url} title={p.tab} active={!explorerActive} selected={selected}>
          {p.page}
        </Browser98>
      </div>
      <div style={{opacity: explorerIn}}>
        <Explorer98 {...E} folder={p.folder} files={files} active={explorerActive} pane={pane} />
      </div>
      {p.copy.mode === 'menu' ? (
        <Menu98
          x={target.x + 4}
          y={target.y + 4}
          at={COPY_CLICK - 11}
          until={COPY_CLICK + 15}
          items={p.copy.items}
          highlight={p.copy.highlight}
          highlightAt={COPY_CLICK + 6}
        />
      ) : null}
      {p.steps.length ? (
        <ProgressDialog98 x={E.x + E.w - 300} y={E.y + E.h - 168} folder={p.folder} steps={p.steps} start={PROGRESS} end={p.resultAt - 1} />
      ) : null}
      {p.extra98}
      {p.copy.mode === 'address' ? <Keys98 at={COPY_CLICK + 6} keys={['Ctrl', 'C']} x={B.x + B.w / 2} y={B.y + B.h / 2 + 20} /> : null}
      <Keys98 at={PASTE} keys={['Ctrl', 'V']} x={E.x + E.w / 2 + 60} y={E.y + E.h / 2 + 10} />
      <DosBar {...D} at={p.resultAt + 14} cmd={p.cmd} />
      <Taskbar
        trayBlink={frame >= PASTE && frame < p.resultAt}
        tasks={[
          {label: p.tab, icon: <GlobeIcon size={14} />, active: !explorerActive},
          {label: p.folder, icon: <FolderIcon size={14} />, active: explorerActive},
        ]}
      />
      <Balloon at={p.resultAt + 4} title={p.toast.title} body={p.toast.body} />
      <Cursor98 keys={keys} busy={[PASTE + 6, p.resultAt]} />
    </Stage>
  );
};
