import React from 'react';
import {AbsoluteFill, Easing, Img, interpolate, spring, staticFile, useCurrentFrame, useVideoConfig} from 'remotion';
import {clamp} from '../ui';

/*
 * Everything in this theme is laid out at half resolution with authentic
 * Windows 98 metrics (11px UI font, 18px title bars, 1px bevels) and the whole
 * stage is scaled 2x, so 1920x1080 is a 960x540 desktop and the reel is 540x960.
 */

export const ui = '"Microsoft Sans Serif", "MS Sans Serif", Tahoma, Arial, sans-serif';
export const dos = '"Lucida Console", "Courier New", monospace';

export const k = {
  desktop: '#008080',
  face: '#c0c0c0',
  shadow: '#808080',
  dark: '#0a0a0a',
  light: '#dfdfdf',
  white: '#ffffff',
  navy: '#000080',
  title: 'linear-gradient(90deg, #000080, #1084d0)',
  titleOff: 'linear-gradient(90deg, #808080, #b5b5b5)',
  tip: '#ffffe1',
};

export const raised = `inset -1px -1px ${k.dark}, inset 1px 1px ${k.light}, inset -2px -2px ${k.shadow}, inset 2px 2px ${k.white}`;
export const sunken = `inset -1px -1px ${k.white}, inset 1px 1px ${k.shadow}, inset -2px -2px ${k.light}, inset 2px 2px ${k.dark}`;
export const pressed = `inset -1px -1px ${k.white}, inset 1px 1px ${k.dark}, inset -2px -2px ${k.light}, inset 2px 2px ${k.shadow}`;
export const thin = `inset -1px -1px ${k.shadow}, inset 1px 1px ${k.white}`;
export const thinSunken = `inset -1px -1px ${k.white}, inset 1px 1px ${k.shadow}`;

export type Rect = {x: number; y: number; w: number; h: number};

export const useStage = () => {
  const {width, height} = useVideoConfig();
  return {w: width / 2, h: height / 2, vertical: height > width};
};

export const Stage: React.FC<{children: React.ReactNode; background?: string}> = ({children, background = k.desktop}) => {
  const {width, height} = useVideoConfig();
  return (
    <AbsoluteFill style={{background}}>
      <div
        style={{
          position: 'absolute',
          left: 0,
          top: 0,
          width: width / 2,
          height: height / 2,
          transform: 'scale(2)',
          transformOrigin: '0 0',
          overflow: 'hidden',
          fontFamily: ui,
          fontSize: 11,
          color: '#000',
        }}
      >
        {children}
      </div>
    </AbsoluteFill>
  );
};

export const Logo98: React.FC<{size: number}> = ({size}) => (
  <Img src={staticFile('logo.png')} style={{width: size, height: size, display: 'block'}} />
);

/* ---------- Icons (32px grid, crisp edges) ---------- */

const svg = (size: number, children: React.ReactNode) => (
  <svg width={size} height={size} viewBox="0 0 32 32" shapeRendering="crispEdges" style={{display: 'block'}}>
    {children}
  </svg>
);

export const FolderIcon: React.FC<{size?: number; git?: boolean}> = ({size = 32, git}) =>
  svg(
    size,
    <>
      <path d="M2 7h10l2 3h16v17H2z" fill="#f8d870" stroke="#000" />
      <path d="M2 12h28v15H2z" fill="#ffe89a" stroke="#000" />
      <path d="M3 13h26" stroke="#fff" />
      {git ? (
        <g transform="translate(17 16)">
          <rect x="0" y="0" width="14" height="14" fill="#f05133" stroke="#000" />
          <path d="M4 3v8M4 7h3l3-3M10 4v0" stroke="#fff" strokeWidth="1.6" fill="none" />
          <rect x="2.5" y="2" width="3" height="3" fill="#fff" />
          <rect x="2.5" y="9.5" width="3" height="3" fill="#fff" />
          <rect x="8.5" y="2.5" width="3" height="3" fill="#fff" />
        </g>
      ) : null}
    </>,
  );

export const DocIcon: React.FC<{size?: number; label?: string; color?: string; lines?: boolean}> = ({size = 32, label, color = '#000080', lines = true}) =>
  svg(
    size,
    <>
      <path d="M6 2h14l6 6v22H6z" fill="#fff" stroke="#000" />
      <path d="M20 2v6h6" fill="#c0c0c0" stroke="#000" />
      {lines ? [12, 15, 18].map((y) => <path key={y} d={`M9 ${y}h13`} stroke="#808080" />) : null}
      {label ? (
        <>
          <rect x="3" y="20" width={Math.max(14, label.length * 5 + 4)} height="9" fill={color} stroke="#000" />
          <text x="5" y="27.5" fontSize="7.5" fontFamily={ui} fontWeight={700} fill="#fff">
            {label}
          </text>
        </>
      ) : null}
    </>,
  );

export const ComputerIcon: React.FC<{size?: number}> = ({size = 32}) =>
  svg(
    size,
    <>
      <rect x="4" y="3" width="24" height="18" fill="#c0c0c0" stroke="#000" />
      <rect x="7" y="6" width="18" height="12" fill="#008080" stroke="#000" />
      <rect x="10" y="21" width="12" height="3" fill="#808080" stroke="#000" />
      <rect x="3" y="24" width="26" height="5" fill="#c0c0c0" stroke="#000" />
      <path d="M20 26h6" stroke="#000" />
    </>,
  );

export const BinIcon: React.FC<{size?: number}> = ({size = 32}) =>
  svg(
    size,
    <>
      <path d="M7 8h18l-2 21H9z" fill="#dfdfdf" stroke="#000" />
      <path d="M5 6h22v3H5z" fill="#c0c0c0" stroke="#000" />
      <path d="M12 11v15M16 11v15M20 11v15" stroke="#808080" />
    </>,
  );

export const GlobeIcon: React.FC<{size?: number}> = ({size = 32}) =>
  svg(
    size,
    <>
      <circle cx="16" cy="16" r="12" fill="#3a7bd5" stroke="#000" shapeRendering="geometricPrecision" />
      <path d="M8 10c4 1 5 4 3 7s1 5 3 6M18 5c-2 3 1 5 4 5s5 3 3 7" stroke="#2ea84a" strokeWidth="3" fill="none" shapeRendering="geometricPrecision" />
      <ellipse cx="16" cy="16" rx="5" ry="12" fill="none" stroke="#000" strokeOpacity={0.35} shapeRendering="geometricPrecision" />
    </>,
  );

export const NotepadIcon: React.FC<{size?: number}> = ({size = 32}) =>
  svg(
    size,
    <>
      <rect x="6" y="4" width="20" height="25" fill="#fff" stroke="#000" />
      <rect x="6" y="4" width="20" height="4" fill="#3a7bd5" stroke="#000" />
      {[12, 15, 18, 21, 24].map((y) => (
        <path key={y} d={`M9 ${y}h14`} stroke="#000" strokeOpacity={0.6} />
      ))}
    </>,
  );

export const DosIcon: React.FC<{size?: number}> = ({size = 32}) =>
  svg(
    size,
    <>
      <rect x="3" y="5" width="26" height="22" fill="#000" stroke="#c0c0c0" />
      <text x="6" y="19" fontSize="9" fontFamily={dos} fill="#c0c0c0">
        C:\
      </text>
    </>,
  );

export type Kind98 = 'folder' | 'repo' | 'video' | 'gif' | 'image' | 'md' | 'txt' | 'zip' | 'pdf';

const badge: Record<string, string> = {MP4: '#a01818', GIF: '#c06000', PNG: '#1060a0', JPG: '#1060a0', MD: '#600080', TXT: '#404040', ZIP: '#207020'};

export const FileIcon98: React.FC<{kind: Kind98; name: string; size?: number}> = ({kind, name, size = 32}) => {
  if (kind === 'folder' || kind === 'repo') return <FolderIcon size={size} git={kind === 'repo'} />;
  const ext = name.includes('.') ? name.split('.').pop()!.toUpperCase() : 'FILE';
  return <DocIcon size={size} label={ext} color={badge[ext] ?? '#000080'} />;
};

/* ---------- Window chrome ---------- */

const TitleButton: React.FC<{glyph: string}> = ({glyph}) => (
  <div
    style={{
      width: 16,
      height: 14,
      background: k.face,
      boxShadow: raised,
      display: 'grid',
      placeItems: 'center',
      fontSize: glyph === '✕' ? 9 : 9,
      fontWeight: 700,
      lineHeight: 1,
      color: '#000',
    }}
  >
    {glyph}
  </div>
);

export const Window: React.FC<
  Rect & {
    title: string;
    icon?: React.ReactNode;
    active?: boolean;
    children: React.ReactNode;
    style?: React.CSSProperties;
    help?: boolean;
  }
> = ({x, y, w, h, title, icon, active = true, children, style, help}) => (
  <div
    style={{
      position: 'absolute',
      left: x,
      top: y,
      width: w,
      height: h,
      background: k.face,
      boxShadow: raised,
      padding: 3,
      display: 'flex',
      flexDirection: 'column',
      ...style,
    }}
  >
    <div
      style={{
        height: 18,
        flexShrink: 0,
        background: active ? k.title : k.titleOff,
        color: active ? '#fff' : '#d4d0c8',
        display: 'flex',
        alignItems: 'center',
        gap: 3,
        padding: '0 2px 0 3px',
        fontWeight: 700,
      }}
    >
      {icon ? <div style={{width: 16, height: 16, display: 'grid', placeItems: 'center'}}>{icon}</div> : null}
      <div style={{flex: 1, whiteSpace: 'nowrap', overflow: 'hidden', textOverflow: 'ellipsis'}}>{title}</div>
      {help ? (
        <TitleButton glyph="?" />
      ) : (
        <>
          <TitleButton glyph="_" />
          <TitleButton glyph="□" />
        </>
      )}
      <div style={{width: 2}} />
      <TitleButton glyph="✕" />
    </div>
    {children}
  </div>
);

export const MenuBar: React.FC<{items: string[]; right?: React.ReactNode}> = ({items, right}) => (
  <div style={{height: 19, display: 'flex', alignItems: 'center', gap: 0, padding: '0 2px', flexShrink: 0}}>
    {items.map((m) => (
      <span key={m} style={{padding: '0 6px'}}>
        <u>{m[0]}</u>
        {m.slice(1)}
      </span>
    ))}
    <div style={{flex: 1}} />
    {right}
  </div>
);

export const Button98: React.FC<{children: React.ReactNode; w?: number; down?: boolean; style?: React.CSSProperties}> = ({children, w = 75, down, style}) => (
  <div
    style={{
      minWidth: w,
      height: 23,
      padding: '0 8px',
      background: k.face,
      boxShadow: down ? pressed : raised,
      display: 'grid',
      placeItems: 'center',
      whiteSpace: 'nowrap',
      ...style,
    }}
  >
    {children}
  </div>
);

/* ---------- Browser (classic web browser) ---------- */

export const BROWSER_CHROME = 100;

const ToolButton: React.FC<{label: string; icon: React.ReactNode; dim?: boolean}> = ({label, icon, dim}) => (
  <div style={{width: 46, display: 'flex', flexDirection: 'column', alignItems: 'center', gap: 1, color: dim ? '#808080' : '#000'}}>
    <div style={{height: 20, display: 'grid', placeItems: 'center', filter: dim ? 'grayscale(1) opacity(0.5)' : undefined}}>{icon}</div>
    <span style={{fontSize: 10}}>{label}</span>
  </div>
);

const Arrow: React.FC<{dir: 'l' | 'r'; color?: string}> = ({dir, color = '#208020'}) => (
  <svg width={18} height={18} viewBox="0 0 18 18" shapeRendering="crispEdges">
    <path d={dir === 'l' ? 'M2 9l7-7v4h7v6H9v4z' : 'M16 9L9 2v4H2v6h7v4z'} fill={color} stroke="#000" />
  </svg>
);

export const Browser98: React.FC<
  Rect & {
    url: string;
    title: string;
    active?: boolean;
    selected?: number;
    busy?: boolean;
    children: React.ReactNode;
  }
> = ({x, y, w, h, url, title, active, selected = 0, children}) => {
  const frame = useCurrentFrame();
  const contentW = w - 6;
  const contentH = h - BROWSER_CHROME - 3 - 20;
  return (
    <Window x={x} y={y} w={w} h={h} title={`${title} - Web Browser`} icon={<GlobeIcon size={14} />} active={active}>
      <MenuBar
        items={['File', 'Edit', 'View', 'Favorites', 'Tools', 'Help']}
        right={
          <div style={{width: 30, height: 18, background: '#000', display: 'grid', placeItems: 'center', boxShadow: thinSunken}}>
            <div style={{transform: `rotate(${frame * 3}deg)`}}>
              <Logo98 size={12} />
            </div>
          </div>
        }
      />
      <div style={{height: 36, display: 'flex', alignItems: 'center', gap: 2, padding: '0 4px', boxShadow: thin, flexShrink: 0}}>
        <ToolButton label="Back" icon={<Arrow dir="l" />} />
        <ToolButton label="Forward" icon={<Arrow dir="r" />} dim />
        <ToolButton label="Stop" icon={<div style={{width: 14, height: 14, background: '#c02020', border: '1px solid #000', transform: 'rotate(45deg) scale(0.8)'}} />} />
        <ToolButton label="Refresh" icon={<div style={{width: 13, height: 13, borderRadius: '50%', border: '3px solid #2060c0', borderRightColor: 'transparent'}} />} />
        <ToolButton label="Home" icon={<svg width={18} height={18} viewBox="0 0 18 18"><path d="M2 9l7-7 7 7v7H2z" fill="#f8d870" stroke="#000" /><rect x="7" y="11" width="4" height="5" fill="#a05020" /></svg>} />
        <div style={{width: 2, height: 28, boxShadow: thinSunken, margin: '0 3px'}} />
        <ToolButton label="Search" icon={<GlobeIcon size={18} />} />
        <ToolButton label="Favorites" icon={<span style={{color: '#e0b000', fontSize: 16}}>★</span>} />
      </div>
      <div style={{height: 22, display: 'flex', alignItems: 'center', gap: 4, padding: '0 4px', boxShadow: thin, flexShrink: 0}}>
        <span style={{color: '#000'}}>A<u>d</u>dress</span>
        <div style={{flex: 1, height: 18, background: '#fff', boxShadow: sunken, display: 'flex', alignItems: 'center', gap: 3, padding: '0 3px', overflow: 'hidden'}}>
          <GlobeIcon size={13} />
          <span
            style={{
              whiteSpace: 'nowrap',
              background: selected > 0.5 ? k.navy : 'transparent',
              color: selected > 0.5 ? '#fff' : '#000',
              padding: '0 1px',
            }}
          >
            https://{url}
          </span>
        </div>
        <Button98 w={30} style={{height: 18}}>
          Go
        </Button98>
      </div>
      <div style={{flex: 1, boxShadow: sunken, padding: 2, position: 'relative', overflow: 'hidden', background: '#fff'}}>
        <div style={{position: 'absolute', left: 2, top: 2, width: (contentW - 4) * 2, height: contentH * 2, transform: 'scale(0.5)', transformOrigin: '0 0', overflow: 'hidden'}}>
          {children}
        </div>
      </div>
      <div style={{height: 20, display: 'flex', alignItems: 'center', gap: 2, paddingTop: 2, flexShrink: 0}}>
        <div style={{flex: 1, height: 18, boxShadow: thinSunken, display: 'flex', alignItems: 'center', padding: '0 4px', gap: 4}}>
          <GlobeIcon size={12} /> Done
        </div>
        <div style={{width: 110, height: 18, boxShadow: thinSunken, display: 'flex', alignItems: 'center', padding: '0 4px', gap: 4}}>
          <GlobeIcon size={12} /> Internet
        </div>
      </div>
    </Window>
  );
};

/* ---------- Explorer ---------- */

export type Item98 = {name: string; kind: Kind98; meta?: string; thumb?: React.ReactNode; appearAt?: number};

const kindName: Record<Kind98, string> = {
  folder: 'File Folder',
  repo: 'Git Repository',
  video: 'MP4 Video',
  gif: 'GIF Image',
  image: 'JPEG Image',
  md: 'Markdown Document',
  txt: 'Text Document',
  zip: 'ZIP Archive',
  pdf: 'PDF Document',
};

export const EXPLORER_BODY = 100;

const Tile98: React.FC<{item: Item98; selected: boolean}> = ({item, selected}) => {
  const frame = useCurrentFrame();
  const {fps} = useVideoConfig();
  if (item.appearAt !== undefined && frame < item.appearAt) return null;
  const pop = item.appearAt === undefined ? 1 : spring({frame: frame - item.appearAt, fps, config: {damping: 12, mass: 0.5}});
  return (
    <div style={{width: 78, display: 'flex', flexDirection: 'column', alignItems: 'center', gap: 3, paddingTop: 4, transform: `scale(${pop})`}}>
      {item.thumb ? (
        <div style={{width: 52, height: 40, position: 'relative', overflow: 'hidden', border: '1px solid #000', outline: selected ? `2px solid ${k.navy}` : 'none', background: '#fff'}}>
          {item.thumb}
        </div>
      ) : (
        <div style={{filter: selected ? 'drop-shadow(0 0 0 #000080) brightness(0.75) sepia(1) hue-rotate(190deg) saturate(3)' : undefined}}>
          <FileIcon98 kind={item.kind} name={item.name} />
        </div>
      )}
      <div
        style={{
          textAlign: 'center',
          lineHeight: '13px',
          padding: '0 2px',
          background: selected ? k.navy : 'transparent',
          color: selected ? '#fff' : '#000',
          outline: selected ? '1px dotted #ffff80' : 'none',
          maxHeight: 40,
          overflow: 'hidden',
          wordBreak: 'break-word',
        }}
      >
        {item.name}
      </div>
    </div>
  );
};

export const Explorer98: React.FC<Rect & {folder: string; files: Item98[]; active?: boolean; pane?: number}> = ({x, y, w, h, folder, files, active, pane = 132}) => {
  const frame = useCurrentFrame();
  const visible = files.filter((f) => f.appearAt === undefined || frame >= f.appearAt);
  const fresh = files.find((f) => f.appearAt !== undefined && frame >= f.appearAt);
  return (
    <Window x={x} y={y} w={w} h={h} title={`C:\\My Documents\\${folder}`} icon={<FolderIcon size={14} />} active={active}>
      <MenuBar items={['File', 'Edit', 'View', 'Go', 'Favorites', 'Help']} />
      <div style={{height: 36, display: 'flex', alignItems: 'center', gap: 2, padding: '0 4px', boxShadow: thin, flexShrink: 0}}>
        <ToolButton label="Back" icon={<Arrow dir="l" />} />
        <ToolButton label="Up" icon={<FolderIcon size={18} />} />
        <div style={{width: 2, height: 28, boxShadow: thinSunken, margin: '0 3px'}} />
        <ToolButton label="Cut" icon={<span style={{fontSize: 15}}>✂</span>} />
        <ToolButton label="Copy" icon={<DocIcon size={18} lines={false} />} />
        <ToolButton label="Paste" icon={<NotepadIcon size={18} />} />
      </div>
      <div style={{height: 22, display: 'flex', alignItems: 'center', gap: 4, padding: '0 4px', boxShadow: thin, flexShrink: 0}}>
        <span>A<u>d</u>dress</span>
        <div style={{flex: 1, height: 18, background: '#fff', boxShadow: sunken, display: 'flex', alignItems: 'center', gap: 3, padding: '0 3px'}}>
          <FolderIcon size={13} /> C:\My Documents\{folder}
        </div>
      </div>
      <div style={{flex: 1, boxShadow: sunken, padding: 2, display: 'flex', background: '#fff', minHeight: 0}}>
        <div style={{width: pane, padding: '10px 8px', position: 'relative', overflow: 'hidden', background: 'linear-gradient(180deg,#fff 60%,#e8f0ff)'}}>
          <FolderIcon size={32} />
          <div style={{fontSize: 17, fontWeight: 700, marginTop: 4, lineHeight: 1.1, wordBreak: 'break-word'}}>{folder}</div>
          <div style={{height: 3, margin: '6px 0 8px', background: 'linear-gradient(90deg,#000080,#1084d0 50%,#ffffff)'}} />
          {fresh ? (
            <div style={{lineHeight: '14px'}}>
              <b style={{wordBreak: 'break-word'}}>{fresh.name}</b>
              <br />
              {kindName[fresh.kind]}
              <br />
              {fresh.meta ? <>{fresh.kind === 'image' ? 'Dimensions' : fresh.kind === 'repo' ? 'Type' : 'Size'}: {fresh.meta}</> : null}
              <br />
              <span style={{color: '#808080'}}>Pasted by JustPaste</span>
            </div>
          ) : (
            <div style={{color: '#000', lineHeight: '14px'}}>Select an item to view its description.</div>
          )}
        </div>
        <div style={{flex: 1, display: 'flex', flexWrap: 'wrap', alignContent: 'flex-start', gap: 4, padding: 8}}>
          {files.map((f) => (
            <Tile98 key={f.name} item={f} selected={f === fresh} />
          ))}
        </div>
      </div>
      <div style={{height: 20, display: 'flex', gap: 2, paddingTop: 2, flexShrink: 0}}>
        <div style={{flex: 1, boxShadow: thinSunken, display: 'flex', alignItems: 'center', padding: '0 4px'}}>
          {fresh ? '1 object(s) selected' : `${visible.length} object(s)`}
        </div>
        <div style={{width: 110, boxShadow: thinSunken, display: 'flex', alignItems: 'center', padding: '0 4px', gap: 4}}>
          <ComputerIcon size={12} /> My Computer
        </div>
      </div>
    </Window>
  );
};

/* ---------- JustPaste progress dialog (flying paper) ---------- */

export type Step98 = {at: number; text: string; detail?: string; pct?: [number, number]};

export const ProgressDialog98: React.FC<{x: number; y: number; folder: string; steps: Step98[]; start: number; end: number}> = ({x, y, folder, steps, start, end}) => {
  const frame = useCurrentFrame();
  if (frame < start || frame > end) return null;
  const current = [...steps].reverse().find((s) => frame >= s.at) ?? steps[0];
  const next = steps[steps.indexOf(current) + 1];
  const pct = current.pct
    ? interpolate(frame, [current.at, next ? next.at - 2 : end - 4], current.pct, {...clamp, easing: Easing.inOut(Easing.quad)})
    : null;
  const t = ((frame - start) % 22) / 22;
  const paperX = 34 + t * 176;
  const paperY = 22 - Math.sin(Math.PI * t) * 16;
  return (
    <Window x={x} y={y} w={290} h={156} title="JustPaste" icon={<Logo98 size={13} />} help style={{zIndex: 4}}>
      <div style={{flex: 1, padding: '6px 10px', display: 'flex', flexDirection: 'column', gap: 5}}>
        <div style={{height: 40, position: 'relative'}}>
          <div style={{position: 'absolute', left: 4, top: 6}}>
            <GlobeIcon />
          </div>
          <div style={{position: 'absolute', right: 4, top: 6}}>
            <FolderIcon />
          </div>
          <div style={{position: 'absolute', left: paperX, top: paperY, transform: `rotate(${t * 40 - 20}deg)`}}>
            <DocIcon size={18} lines />
          </div>
        </div>
        <div style={{whiteSpace: 'nowrap', overflow: 'hidden', textOverflow: 'ellipsis'}}>{current.text}</div>
        <div style={{color: '#000'}}>To: C:\My Documents\{folder}</div>
        <div style={{height: 16, background: '#fff', boxShadow: sunken, padding: 2, overflow: 'hidden', position: 'relative'}}>
          {pct === null ? (
            <div
              style={{
                position: 'absolute',
                top: 2,
                bottom: 2,
                width: 40,
                left: 2 + (((frame - start) * 4) % 300) - 40,
                background: `repeating-linear-gradient(90deg, ${k.navy} 0 8px, transparent 8px 10px)`,
              }}
            />
          ) : (
            <div
              style={{
                height: '100%',
                width: `${Math.floor(pct / 4) * 4}%`,
                background: `repeating-linear-gradient(90deg, ${k.navy} 0 8px, transparent 8px 10px)`,
              }}
            />
          )}
        </div>
        <div style={{display: 'flex', justifyContent: 'space-between', alignItems: 'center'}}>
          <span>
            {pct !== null ? `${Math.round(pct)}%` : ''}
            {current.detail ? `${pct !== null ? ' · ' : ''}${current.detail}` : ''}
          </span>
          <Button98 w={80}>Cancel paste</Button98>
        </div>
      </div>
    </Window>
  );
};

/* ---------- Taskbar ---------- */

export const Taskbar: React.FC<{tasks: {label: string; icon: React.ReactNode; active: boolean}[]; trayBlink?: boolean}> = ({tasks, trayBlink}) => {
  const frame = useCurrentFrame();
  const {w, h, vertical} = useStage();
  return (
    <div
      style={{
        position: 'absolute',
        left: 0,
        top: h - 28,
        width: w,
        height: 28,
        background: k.face,
        boxShadow: `inset 0 1px ${k.light}, inset 0 2px ${k.white}`,
        display: 'flex',
        alignItems: 'center',
        gap: 3,
        padding: '2px 2px 0',
        zIndex: 20,
      }}
    >
      <div style={{height: 22, padding: '0 5px', boxShadow: raised, display: 'flex', alignItems: 'center', gap: 4, fontWeight: 700}}>
        <Logo98 size={15} />
        Start
      </div>
      <div style={{width: 2, height: 20, boxShadow: thinSunken, margin: '0 2px'}} />
      {!vertical ? (
        <>
          <GlobeIcon size={16} />
          <DosIcon size={16} />
          <div style={{width: 2, height: 20, boxShadow: thinSunken, margin: '0 2px'}} />
        </>
      ) : null}
      {tasks.map((t) => (
        <div
          key={t.label}
          style={{
            width: vertical ? 150 : 160,
            height: 22,
            boxShadow: t.active ? pressed : raised,
            background: t.active ? `repeating-conic-gradient(#c0c0c0 0 25%, #fff 0 50%) 0 0 / 2px 2px` : k.face,
            display: 'flex',
            alignItems: 'center',
            gap: 4,
            padding: '0 4px',
            fontWeight: t.active ? 700 : 400,
            whiteSpace: 'nowrap',
            overflow: 'hidden',
          }}
        >
          <div style={{flexShrink: 0}}>{t.icon}</div>
          <span style={{overflow: 'hidden', textOverflow: 'ellipsis'}}>{t.label}</span>
        </div>
      ))}
      <div style={{flex: 1}} />
      <div style={{height: 22, boxShadow: thinSunken, display: 'flex', alignItems: 'center', gap: 5, padding: '0 6px'}}>
        <div style={{opacity: trayBlink && Math.floor(frame / 6) % 2 ? 0.35 : 1}}>
          <Logo98 size={15} />
        </div>
        <span style={{fontSize: 12}}>🔊</span>
        <span>3:14 PM</span>
      </div>
    </div>
  );
};

export const TRAY_ICON_FROM_RIGHT = 82;

/* ---------- Balloon tip ---------- */

export const Balloon: React.FC<{at: number; title: string; body: string; dur?: number}> = ({at, title, body, dur = 80}) => {
  const frame = useCurrentFrame();
  const {w, h} = useStage();
  if (frame < at || frame > at + dur) return null;
  const s = interpolate(frame, [at, at + 5], [0, 1], clamp);
  const width = 250;
  const tipX = w - TRAY_ICON_FROM_RIGHT;
  return (
    <div style={{position: 'absolute', left: Math.min(w - width - 6, tipX - width + 40), top: h - 28 - 62, width, opacity: s, zIndex: 25}}>
      <div style={{background: k.tip, border: '1px solid #000', borderRadius: 7, padding: '6px 8px', boxShadow: '2px 2px 0 rgba(0,0,0,0.25)'}}>
        <div style={{display: 'flex', alignItems: 'center', gap: 5, fontWeight: 700}}>
          <Logo98 size={14} />
          <span style={{flex: 1}}>{title}</span>
          <span style={{fontSize: 9, border: '1px solid #808080', padding: '0 2px', lineHeight: '10px'}}>✕</span>
        </div>
        <div style={{marginTop: 3, whiteSpace: 'nowrap', overflow: 'hidden', textOverflow: 'ellipsis'}}>{body}</div>
      </div>
      <svg width={14} height={10} style={{position: 'absolute', left: tipX - Math.min(w - width - 6, tipX - width + 40) - 7, top: '100%', marginTop: -1}}>
        <path d="M0 0 L7 9 L12 0" fill={k.tip} stroke="#000" />
        <path d="M1 0 H11" stroke={k.tip} strokeWidth={2} />
      </svg>
    </div>
  );
};

/* ---------- Context menu ---------- */

export const Menu98: React.FC<{x: number; y: number; at: number; until: number; items: string[]; highlight: number; highlightAt: number}> = ({x, y, at, until, items, highlight, highlightAt}) => {
  const frame = useCurrentFrame();
  if (frame < at || frame > until) return null;
  const open = interpolate(frame, [at, at + 4], [0, 1], clamp);
  return (
    <div
      style={{
        position: 'absolute',
        left: x,
        top: y,
        width: 150,
        background: k.face,
        boxShadow: raised,
        padding: 3,
        zIndex: 10,
        clipPath: `inset(0 ${(1 - open) * 100}% ${(1 - open) * 100}% 0)`,
      }}
    >
      {items.map((it, i) =>
        it === '-' ? (
          <div key={i} style={{height: 2, margin: '3px 1px', boxShadow: `inset 0 1px ${k.shadow}, inset 0 -1px ${k.white}`}} />
        ) : (
          <div
            key={i}
            style={{
              height: 17,
              lineHeight: '17px',
              padding: '0 18px',
              background: i === highlight && frame >= highlightAt ? k.navy : 'transparent',
              color: i === highlight && frame >= highlightAt ? '#fff' : '#000',
            }}
          >
            {it}
          </div>
        ),
      )}
    </div>
  );
};

export const MENU_ROW = 17;

/* ---------- Cursor ---------- */

export type Key98 = {f: number; x: number; y: number; click?: boolean};

export const Cursor98: React.FC<{keys: Key98[]; busy?: [number, number]}> = ({keys, busy}) => {
  const frame = useCurrentFrame();
  let x = keys[0].x;
  let y = keys[0].y;
  for (let i = 0; i < keys.length - 1; i++) {
    const a = keys[i];
    const b = keys[i + 1];
    if (frame >= a.f && frame <= b.f) {
      const t = interpolate(frame, [a.f, b.f], [0, 1], {...clamp, easing: Easing.bezier(0.45, 0, 0.2, 1)});
      x = a.x + (b.x - a.x) * t;
      y = a.y + (b.y - a.y) * t;
    }
  }
  const last = keys[keys.length - 1];
  if (frame > last.f) {
    x = last.x;
    y = last.y;
  }
  const isBusy = busy ? frame >= busy[0] && frame < busy[1] : false;
  return (
    <div style={{position: 'absolute', left: x, top: y, zIndex: 30, pointerEvents: 'none'}}>
      <svg width={12} height={19} viewBox="0 0 12 19" shapeRendering="crispEdges" style={{display: 'block'}}>
        <path d="M0.5 0.5 L0.5 15.5 L4 12 L6.5 17.5 L8.5 16.5 L6 11.2 L10.8 11.2 Z" fill="#fff" stroke="#000" />
      </svg>
      {isBusy ? (
        <svg width={10} height={14} viewBox="0 0 10 14" shapeRendering="crispEdges" style={{position: 'absolute', left: 10, top: 9, transform: `rotate(${Math.floor(frame / 8) % 2 ? 0 : 180}deg)`}}>
          <path d="M0.5 0.5h9v2L6 7l3.5 4.5v2h-9v-2L4 7 0.5 2.5z" fill="#fff" stroke="#000" />
          <path d="M2.5 2.5h5L5 6zM2.5 12L5 9.5 7.5 12z" fill="#000" />
        </svg>
      ) : null}
    </div>
  );
};

/* ---------- Beige keycaps ---------- */

export const Keys98: React.FC<{at: number; keys: string[]; x: number; y: number; dur?: number}> = ({at, keys, x, y, dur = 30}) => {
  const frame = useCurrentFrame();
  const {fps} = useVideoConfig();
  if (frame < at || frame > at + dur + 8) return null;
  const s = spring({frame: frame - at, fps, config: {damping: 12, mass: 0.5}});
  const out = interpolate(frame, [at + dur, at + dur + 8], [1, 0], clamp);
  const press = interpolate(frame - at, [6, 9, 14], [0, 1, 0], clamp);
  return (
    <div style={{position: 'absolute', left: x, top: y, display: 'flex', gap: 6, alignItems: 'center', transform: `translate(-50%,-50%) scale(${s})`, opacity: out, zIndex: 15}}>
      {keys.map((key, i) => (
        <React.Fragment key={key}>
          {i > 0 ? <span style={{color: '#fff', fontSize: 18, fontWeight: 700, textShadow: '1px 1px 0 #000'}}>+</span> : null}
          <div
            style={{
              minWidth: 44,
              height: 44,
              padding: '0 10px',
              background: 'linear-gradient(180deg,#f2ecd9,#d8cfb4)',
              border: '1px solid #6b6450',
              borderRadius: 4,
              boxShadow: `inset 2px 2px 0 #fffaf0, inset -2px -3px 0 #b3a98c, 0 ${4 - press * 3}px 0 #6b6450`,
              transform: `translateY(${press * 3}px)`,
              display: 'grid',
              placeItems: 'center',
              fontSize: 16,
              fontWeight: 700,
              color: '#2a2618',
            }}
          >
            {key}
          </div>
        </React.Fragment>
      ))}
    </div>
  );
};

/* ---------- DOS prompt strip ---------- */

export const DosBar: React.FC<Rect & {at: number; cmd: string}> = ({x, y, w, h, at, cmd}) => {
  const frame = useCurrentFrame();
  if (frame < at) return null;
  const open = interpolate(frame, [at, at + 5], [0, 1], clamp);
  const prompt = 'C:\\>';
  const typed = Math.floor(interpolate(frame, [at + 5, at + 5 + cmd.length * 0.55], [0, cmd.length], clamp));
  return (
    <div style={{opacity: open}}>
      <Window x={x} y={y} w={w} h={h} title="MS-DOS Prompt — Headless" icon={<DosIcon size={14} />} style={{zIndex: 12}}>
        <div style={{flex: 1, background: '#000', color: '#c0c0c0', fontFamily: dos, fontSize: 11, padding: '6px 6px', whiteSpace: 'nowrap', overflow: 'hidden'}}>
          {prompt}
          {cmd.slice(0, typed)}
          <span style={{opacity: Math.floor(frame / 8) % 2 ? 1 : 0}}>_</span>
        </div>
      </Window>
    </div>
  );
};

/* ---------- Desktop icons and headers ---------- */

export const DesktopIcon: React.FC<{x: number; y: number; icon: React.ReactNode; label: string}> = ({x, y, icon, label}) => (
  <div style={{position: 'absolute', left: x, top: y, width: 64, display: 'flex', flexDirection: 'column', alignItems: 'center', gap: 3}}>
    {icon}
    <span style={{color: '#fff', textAlign: 'center', lineHeight: '12px'}}>{label}</span>
  </div>
);

export const Desktop: React.FC = () => {
  const {vertical} = useStage();
  if (vertical) return null;
  const icons = [
    {icon: <ComputerIcon />, label: 'My Computer'},
    {icon: <FolderIcon />, label: 'My Documents'},
    {icon: <GlobeIcon />, label: 'Internet'},
    {icon: <BinIcon />, label: 'Recycle Bin'},
    {icon: <Logo98 size={32} />, label: 'JustTools'},
  ];
  return (
    <>
      {icons.map((ic, i) => (
        <DesktopIcon key={ic.label} x={8} y={70 + i * 66} icon={ic.icon} label={ic.label} />
      ))}
    </>
  );
};

export const Header98: React.FC<{index: string; title: string; sub: string}> = ({index, title, sub}) => {
  const frame = useCurrentFrame();
  const {w, vertical} = useStage();
  const a = interpolate(frame, [2, 10], [0, 1], clamp);
  const b = interpolate(frame, [8, 16], [0, 1], clamp);
  return (
    <div style={{position: 'absolute', left: vertical ? 14 : 84, top: vertical ? 30 : 8, width: w - (vertical ? 28 : 100), zIndex: 2}}>
      <div style={{display: 'flex', alignItems: 'center', gap: 8, opacity: a}}>
        <div style={{background: k.face, boxShadow: raised, padding: '2px 6px', fontWeight: 700, fontSize: vertical ? 15 : 12, fontFamily: dos}}>{index}</div>
        <div style={{color: '#fff', fontWeight: 700, fontSize: vertical ? 27 : 23, textShadow: '2px 2px 0 #000', letterSpacing: -0.3}}>{title}</div>
      </div>
      <div style={{color: '#fff', fontSize: vertical ? 15 : 12, marginTop: 5, opacity: b, textShadow: '1px 1px 0 #000', lineHeight: 1.3}}>{sub}</div>
    </div>
  );
};
