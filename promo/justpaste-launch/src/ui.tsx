import React from 'react';
import {AbsoluteFill, Easing, Img, interpolate, spring, staticFile, useCurrentFrame, useVideoConfig} from 'remotion';
import {brandGradient, c, mono, sans} from './theme';

export const clamp = {extrapolateLeft: 'clamp', extrapolateRight: 'clamp'} as const;
export const ease = Easing.bezier(0.22, 1, 0.36, 1);

export const useSpring = (delay: number, damping = 16, mass = 0.7) => {
  const frame = useCurrentFrame();
  const {fps} = useVideoConfig();
  return spring({frame: frame - delay, fps, config: {damping, mass}});
};

/* ---------- Background ---------- */

export const Aurora: React.FC<{hue?: number}> = ({hue = 0}) => {
  const f = useCurrentFrame();
  const a = Math.sin(f / 90) * 120;
  const b = Math.cos(f / 110) * 140;
  return (
    <AbsoluteFill style={{background: c.bg, overflow: 'hidden'}}>
      <div
        style={{
          position: 'absolute',
          width: 1400,
          height: 1400,
          left: -300 + a,
          top: -700 + b * 0.4,
          background: `radial-gradient(circle, rgba(123,77,255,0.32), transparent 62%)`,
          filter: `hue-rotate(${hue}deg)`,
        }}
      />
      <div
        style={{
          position: 'absolute',
          width: 1300,
          height: 1300,
          right: -400 - b,
          bottom: -760 + a * 0.3,
          background: `radial-gradient(circle, rgba(79,125,255,0.28), transparent 62%)`,
          filter: `hue-rotate(${hue}deg)`,
        }}
      />
      <AbsoluteFill
        style={{
          backgroundImage:
            'linear-gradient(rgba(255,255,255,0.035) 1px, transparent 1px), linear-gradient(90deg, rgba(255,255,255,0.035) 1px, transparent 1px)',
          backgroundSize: '64px 64px',
          maskImage: 'radial-gradient(ellipse at center, black 30%, transparent 75%)',
          WebkitMaskImage: 'radial-gradient(ellipse at center, black 30%, transparent 75%)',
        }}
      />
    </AbsoluteFill>
  );
};

export const Logo: React.FC<{size: number; style?: React.CSSProperties}> = ({size, style}) => (
  <Img src={staticFile('logo.png')} style={{width: size, height: size, filter: 'drop-shadow(0 20px 50px rgba(123,77,255,0.55))', ...style}} />
);

/* ---------- Windows ---------- */

const WinControls: React.FC = () => (
  <div style={{display: 'flex', height: '100%'}}>
    {['—', '▢', '✕'].map((g) => (
      <div key={g} style={{width: 46, display: 'grid', placeItems: 'center', color: c.dim, fontSize: g === '▢' ? 13 : 14}}>
        {g}
      </div>
    ))}
  </div>
);

export const Frame: React.FC<{
  x: number;
  y: number;
  w: number;
  h: number;
  focus?: number;
  children: React.ReactNode;
  style?: React.CSSProperties;
}> = ({x, y, w, h, focus = 0, children, style}) => (
  <div
    style={{
      position: 'absolute',
      left: x,
      top: y,
      width: w,
      height: h,
      borderRadius: 14,
      overflow: 'hidden',
      background: '#1b1c22',
      border: `1px solid rgba(255,255,255,${0.1 + focus * 0.1})`,
      boxShadow: `0 40px 120px rgba(0,0,0,0.6), 0 0 0 ${focus * 3}px rgba(104,182,255,${focus * 0.35})`,
      fontFamily: sans,
      color: c.text,
      display: 'flex',
      flexDirection: 'column',
      ...style,
    }}
  >
    {children}
  </div>
);

export const Browser: React.FC<{
  x: number;
  y: number;
  w: number;
  h: number;
  url: string;
  tab: string;
  tabIcon?: React.ReactNode;
  selected?: number;
  focus?: number;
  children: React.ReactNode;
}> = ({x, y, w, h, url, tab, tabIcon, selected = 0, focus = 0, children}) => {
  const [host, ...rest] = url.split('/');
  return (
    <Frame x={x} y={y} w={w} h={h} focus={focus}>
      <div style={{height: 46, display: 'flex', alignItems: 'flex-end', background: '#202127', paddingLeft: 12}}>
        <div
          style={{
            height: 38,
            width: 270,
            background: '#2b2c33',
            borderRadius: '10px 10px 0 0',
            display: 'flex',
            alignItems: 'center',
            gap: 10,
            padding: '0 14px',
            fontSize: 15,
          }}
        >
          <div style={{width: 18, height: 18, display: 'grid', placeItems: 'center'}}>{tabIcon}</div>
          <span style={{whiteSpace: 'nowrap', overflow: 'hidden', textOverflow: 'ellipsis', flex: 1}}>{tab}</span>
          <span style={{color: c.dim, fontSize: 13}}>✕</span>
        </div>
        <div style={{color: c.dim, fontSize: 22, padding: '0 14px 8px'}}>+</div>
        <div style={{flex: 1}} />
        <div style={{alignSelf: 'stretch'}}>
          <WinControls />
        </div>
      </div>
      <div style={{height: 52, background: '#2b2c33', display: 'flex', alignItems: 'center', gap: 16, padding: '0 16px'}}>
        <span style={{color: c.dim, fontSize: 18, letterSpacing: 10}}>←→↻</span>
        <div
          style={{
            flex: 1,
            height: 36,
            borderRadius: 18,
            background: '#1b1c22',
            display: 'flex',
            alignItems: 'center',
            padding: '0 16px',
            gap: 10,
            fontSize: 16,
            boxShadow: selected > 0 ? `inset 0 0 0 2px ${c.accent}` : 'none',
          }}
        >
          <span style={{color: c.dim, fontSize: 13}}>🔒</span>
          <span
            style={{
              position: 'relative',
              padding: '2px 3px',
              borderRadius: 3,
              background: `rgba(64,128,255,${selected * 0.75})`,
              color: selected > 0.5 ? '#fff' : c.text,
            }}
          >
            {selected > 0.5 ? (
              <>https://{url}</>
            ) : (
              <>
                {host}
                <span style={{color: c.dim}}>{rest.length ? '/' + rest.join('/') : ''}</span>
              </>
            )}
          </span>
        </div>
        <div style={{width: 28, height: 28, borderRadius: 14, background: brandGradient}} />
      </div>
      <div style={{flex: 1, position: 'relative', overflow: 'hidden'}}>{children}</div>
    </Frame>
  );
};

/* ---------- Explorer ---------- */

export type FileKind = 'video' | 'gif' | 'folder' | 'repo' | 'image' | 'md' | 'pdf' | 'txt' | 'zip';

export type FileItem = {
  name: string;
  kind: FileKind;
  meta?: string;
  thumb?: React.ReactNode;
  appearAt?: number;
};

const kindColor: Record<FileKind, string> = {
  video: '#ff5f6d',
  gif: '#ff9f43',
  folder: '#f5c451',
  repo: '#f5c451',
  image: '#68b6ff',
  md: '#c291ff',
  pdf: '#ff6b6b',
  txt: '#a0a8b8',
  zip: '#7bd389',
};

export const FileGlyph: React.FC<{kind: FileKind; size?: number; name?: string}> = ({kind, size = 92, name}) => {
  if (kind === 'folder' || kind === 'repo') {
    return (
      <div style={{width: size, height: size * 0.78, position: 'relative'}}>
        <div style={{position: 'absolute', left: 0, top: 0, width: size * 0.42, height: size * 0.2, borderRadius: '8px 8px 0 0', background: '#d9a63a'}} />
        <div
          style={{
            position: 'absolute',
            left: 0,
            top: size * 0.12,
            width: size,
            height: size * 0.66,
            borderRadius: 8,
            background: 'linear-gradient(180deg,#ffd76a,#f0b43c)',
            boxShadow: 'inset 0 2px 0 rgba(255,255,255,0.4)',
          }}
        />
        {kind === 'repo' ? (
          <div
            style={{
              position: 'absolute',
              right: -6,
              bottom: -8,
              width: size * 0.42,
              height: size * 0.42,
              borderRadius: '50%',
              background: '#f05133',
              display: 'grid',
              placeItems: 'center',
              boxShadow: '0 4px 12px rgba(0,0,0,0.4)',
            }}
          >
            <GitMark size={size * 0.26} color="#fff" />
          </div>
        ) : null}
      </div>
    );
  }
  const ext = name?.includes('.') ? name.split('.').pop()!.toUpperCase() : undefined;
  const label = ext ?? {video: 'MP4', gif: 'GIF', image: 'JPG', md: 'MD', pdf: 'PDF', txt: 'TXT', zip: 'ZIP'}[kind];
  return (
    <div
      style={{
        width: size * 0.76,
        height: size,
        borderRadius: 8,
        background: 'linear-gradient(180deg,#f4f5f8,#d9dce4)',
        position: 'relative',
        boxShadow: '0 6px 14px rgba(0,0,0,0.35)',
      }}
    >
      <div
        style={{
          position: 'absolute',
          bottom: size * 0.14,
          left: -6,
          padding: '3px 8px',
          borderRadius: 5,
          fontSize: size * 0.15,
          fontWeight: 800,
          color: '#fff',
          background: kindColor[kind],
          fontFamily: sans,
        }}
      >
        {label}
      </div>
    </div>
  );
};

export const GitMark: React.FC<{size: number; color?: string}> = ({size, color = '#fff'}) => (
  <svg width={size} height={size} viewBox="0 0 24 24" fill="none" stroke={color} strokeWidth={2.4} strokeLinecap="round">
    <circle cx="6" cy="6" r="2.5" />
    <circle cx="6" cy="18" r="2.5" />
    <circle cx="18" cy="9" r="2.5" />
    <path d="M6 8.5v7M18 11.5c0 4-6 3-11 5" />
  </svg>
);

const FileTile: React.FC<{item: FileItem}> = ({item}) => {
  const frame = useCurrentFrame();
  const {fps} = useVideoConfig();
  const appear = item.appearAt === undefined ? 1 : spring({frame: frame - item.appearAt, fps, config: {damping: 11, mass: 0.6}});
  const glow = item.appearAt === undefined ? 0 : interpolate(frame - item.appearAt, [0, 10, 70], [0, 1, 0.35], clamp);
  if (item.appearAt !== undefined && frame < item.appearAt) return null;
  return (
    <div
      style={{
        width: 168,
        height: 196,
        borderRadius: 10,
        display: 'flex',
        flexDirection: 'column',
        alignItems: 'center',
        gap: 12,
        paddingTop: 16,
        transform: `scale(${appear})`,
        background: glow ? `rgba(104,182,255,${0.12 + glow * 0.12})` : 'transparent',
        boxShadow: glow ? `0 0 0 2px rgba(104,182,255,${glow * 0.8}), 0 0 ${50 * glow}px rgba(123,77,255,${glow * 0.6})` : 'none',
      }}
    >
      <div style={{height: 106, display: 'grid', placeItems: 'center'}}>
        {item.thumb ? (
          <div style={{width: 140, height: 100, borderRadius: 8, overflow: 'hidden', position: 'relative', boxShadow: '0 6px 16px rgba(0,0,0,0.45)'}}>
            {item.thumb}
          </div>
        ) : (
          <FileGlyph kind={item.kind} name={item.name} />
        )}
      </div>
      <div style={{fontSize: 15, textAlign: 'center', lineHeight: 1.25, padding: '0 6px', maxHeight: 40, overflow: 'hidden', wordBreak: 'break-word'}}>
        {item.name}
      </div>
      {item.meta ? <div style={{fontSize: 13, color: c.dim, marginTop: -8}}>{item.meta}</div> : null}
    </div>
  );
};

export const Explorer: React.FC<{
  x: number;
  y: number;
  w: number;
  h: number;
  folder: string;
  files: FileItem[];
  focus?: number;
}> = ({x, y, w, h, folder, files, focus = 0}) => {
  const frame = useCurrentFrame();
  const count = files.filter((f) => f.appearAt === undefined || frame >= f.appearAt).length;
  return (
  <Frame x={x} y={y} w={w} h={h} focus={focus} style={{background: '#191a1f'}}>
    <div style={{height: 46, display: 'flex', alignItems: 'center', background: '#202127', paddingLeft: 16, gap: 10, fontSize: 15}}>
      <FileGlyph kind="folder" size={20} />
      <span>{folder}</span>
      <div style={{flex: 1}} />
      <WinControls />
    </div>
    <div style={{height: 48, display: 'flex', alignItems: 'center', gap: 14, padding: '0 16px', background: '#202127', borderBottom: `1px solid ${c.line}`}}>
      <span style={{color: c.dim, fontSize: 17, letterSpacing: 8}}>←→↑</span>
      <div style={{flex: 1, height: 32, borderRadius: 6, background: '#2b2c33', display: 'flex', alignItems: 'center', padding: '0 12px', fontSize: 15, gap: 8}}>
        <span style={{color: c.dim}}>This PC ›</span> <span style={{color: c.dim}}>Projects ›</span> {folder}
      </div>
      <div style={{width: 200, height: 32, borderRadius: 6, background: '#2b2c33', color: c.dim, fontSize: 14, display: 'flex', alignItems: 'center', padding: '0 12px'}}>
        Search {folder}
      </div>
    </div>
    <div style={{height: 44, display: 'flex', alignItems: 'center', gap: 22, padding: '0 18px', fontSize: 14, color: c.dim, borderBottom: `1px solid ${c.line}`}}>
      <span style={{color: c.text}}>⊕ New</span>
      <span>✂</span>
      <span>⧉</span>
      <span style={{color: c.text}}>📋 Paste</span>
      <span>Sort ▾</span>
      <span>View ▾</span>
    </div>
    <div style={{flex: 1, display: 'flex', minHeight: 0}}>
      <div style={{width: 170, borderRight: `1px solid ${c.line}`, padding: '14px 12px', fontSize: 14, color: c.dim, display: 'flex', flexDirection: 'column', gap: 12}}>
        {['Home', 'Desktop', 'Downloads', 'Documents', 'Pictures', 'Projects'].map((n) => (
          <div key={n} style={{display: 'flex', gap: 8, alignItems: 'center', color: n === 'Projects' ? c.text : c.dim}}>
            <FileGlyph kind="folder" size={16} /> {n}
          </div>
        ))}
      </div>
      <div style={{flex: 1, padding: 18, display: 'flex', flexWrap: 'wrap', alignContent: 'flex-start', gap: 8, position: 'relative'}}>
        {files.map((f) => (
          <FileTile key={f.name} item={f} />
        ))}
      </div>
    </div>
    <div style={{height: 30, borderTop: `1px solid ${c.line}`, fontSize: 13, color: c.dim, display: 'flex', alignItems: 'center', padding: '0 16px'}}>
      {count} items
    </div>
  </Frame>
  );
};

/* ---------- JustPaste progress window ---------- */

export type Step = {at: number; text: string; detail?: string; pct?: [number, number]};

export const ProgressWindow: React.FC<{
  x: number;
  y: number;
  folder: string;
  steps: Step[];
  start: number;
  end: number;
}> = ({x, y, folder, steps, start, end}) => {
  const frame = useCurrentFrame();
  const {fps} = useVideoConfig();
  if (frame < start || frame > end + 12) return null;
  const inS = spring({frame: frame - start, fps, config: {damping: 14}});
  const out = interpolate(frame, [end, end + 10], [1, 0], clamp);
  const current = [...steps].reverse().find((s) => frame >= s.at) ?? steps[0];
  const next = steps[steps.indexOf(current) + 1];
  const pct = current.pct
    ? interpolate(frame, [current.at, next ? next.at - 2 : end - 4], current.pct, {...clamp, easing: Easing.inOut(Easing.quad)})
    : null;
  return (
    <div
      style={{
        position: 'absolute',
        left: x,
        top: y,
        width: 470,
        borderRadius: 12,
        background: 'rgba(32,33,40,0.96)',
        border: '1px solid rgba(255,255,255,0.14)',
        boxShadow: '0 30px 80px rgba(0,0,0,0.65), 0 0 60px rgba(123,77,255,0.25)',
        fontFamily: sans,
        color: c.text,
        opacity: out,
        transform: `translateY(${(1 - inS) * 30}px) scale(${0.94 + inS * 0.06})`,
        overflow: 'hidden',
      }}
    >
      <div style={{height: 40, display: 'flex', alignItems: 'center', gap: 10, padding: '0 14px', borderBottom: `1px solid ${c.line}`, fontSize: 14}}>
        <Logo size={20} style={{filter: 'none'}} />
        JustPaste
        <div style={{flex: 1}} />
        <span style={{color: c.dim}}>✕</span>
      </div>
      <div style={{padding: '16px 18px 18px', display: 'flex', flexDirection: 'column', gap: 10}}>
        <div style={{fontSize: 18, fontWeight: 600}}>{current.text}</div>
        <div style={{fontSize: 13, color: c.dim}}>To: C:\Users\you\Projects\{folder}</div>
        <div style={{height: 8, borderRadius: 4, background: '#2f313b', overflow: 'hidden', marginTop: 4}}>
          {pct === null ? (
            <div
              style={{
                width: '30%',
                height: '100%',
                borderRadius: 4,
                background: brandGradient,
                transform: `translateX(${((frame * 9) % 600) - 160}px)`,
              }}
            />
          ) : (
            <div style={{width: `${pct}%`, height: '100%', borderRadius: 4, background: brandGradient}} />
          )}
        </div>
        <div style={{display: 'flex', justifyContent: 'space-between', alignItems: 'center', fontSize: 13, color: c.dim}}>
          <span style={{fontFamily: mono}}>
            {pct !== null ? `${Math.round(pct)}%` : ''}
            {current.detail ? `${pct !== null ? ' · ' : ''}${current.detail}` : ''}
          </span>
          <span style={{border: '1px solid rgba(255,255,255,0.18)', borderRadius: 6, padding: '5px 12px', color: c.text}}>Cancel paste</span>
        </div>
      </div>
    </div>
  );
};

/* ---------- Toast ---------- */

export const Toast: React.FC<{at: number; title: string; body: string; x?: number; y?: number; dur?: number}> = ({at, title, body, x = 1460, y = 880, dur = 75}) => {
  const frame = useCurrentFrame();
  const {fps} = useVideoConfig();
  if (frame < at || frame > at + dur + 12) return null;
  const s = spring({frame: frame - at, fps, config: {damping: 15}});
  const out = interpolate(frame, [at + dur, at + dur + 10], [1, 0], clamp);
  return (
    <div
      style={{
        position: 'absolute',
        left: x,
        top: y,
        width: 440,
        padding: '16px 18px',
        borderRadius: 12,
        background: 'rgba(36,37,45,0.97)',
        border: '1px solid rgba(255,255,255,0.14)',
        boxShadow: '0 24px 70px rgba(0,0,0,0.6)',
        fontFamily: sans,
        color: c.text,
        display: 'flex',
        gap: 14,
        alignItems: 'center',
        opacity: out,
        transform: `translateX(${(1 - s) * 440}px)`,
      }}
    >
      <Logo size={40} style={{filter: 'none'}} />
      <div style={{minWidth: 0}}>
        <div style={{fontSize: 16, fontWeight: 600}}>{title}</div>
        <div style={{fontSize: 14, color: c.dim, marginTop: 3, whiteSpace: 'nowrap', overflow: 'hidden', textOverflow: 'ellipsis'}}>{body}</div>
      </div>
    </div>
  );
};

/* ---------- Cursor ---------- */

export type CursorKey = {f: number; x: number; y: number; click?: boolean};

export const Cursor: React.FC<{keys: CursorKey[]}> = ({keys}) => {
  const frame = useCurrentFrame();
  const frames = keys.map((k) => k.f);
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
  if (frame > frames[frames.length - 1]) {
    x = keys[keys.length - 1].x;
    y = keys[keys.length - 1].y;
  }
  const clicks = keys.filter((k) => k.click);
  const press = clicks.reduce((acc, k) => Math.max(acc, interpolate(frame, [k.f, k.f + 4, k.f + 9], [0, 1, 0], clamp)), 0);
  return (
    <>
      {clicks.map((k) => {
        const t = frame - k.f;
        if (t < 0 || t > 20) return null;
        return (
          <div
            key={k.f}
            style={{
              position: 'absolute',
              left: k.x - 30,
              top: k.y - 30,
              width: 60,
              height: 60,
              borderRadius: '50%',
              border: `3px solid ${c.accent}`,
              opacity: 1 - t / 20,
              transform: `scale(${0.3 + t / 14})`,
            }}
          />
        );
      })}
      <svg
        width={34}
        height={40}
        viewBox="0 0 17 20"
        style={{position: 'absolute', left: x - 3, top: y - 2, transform: `scale(${1 - press * 0.15})`, filter: 'drop-shadow(0 4px 6px rgba(0,0,0,0.5))'}}
      >
        <path d="M1 1 L1 16 L5 12.5 L8 19 L10.5 18 L7.6 11.6 L13 11.6 Z" fill="#fff" stroke="#000" strokeWidth={1.1} strokeLinejoin="round" />
      </svg>
    </>
  );
};

/* ---------- Keycaps ---------- */

export const Keys: React.FC<{at: number; keys: string[]; x: number; y: number; dur?: number}> = ({at, keys, x, y, dur = 30}) => {
  const frame = useCurrentFrame();
  const {fps} = useVideoConfig();
  if (frame < at || frame > at + dur + 10) return null;
  const s = spring({frame: frame - at, fps, config: {damping: 12, mass: 0.5}});
  const out = interpolate(frame, [at + dur, at + dur + 8], [1, 0], clamp);
  const press = interpolate(frame - at, [6, 9, 14], [0, 1, 0], clamp);
  return (
    <div
      style={{
        position: 'absolute',
        left: x,
        top: y,
        display: 'flex',
        gap: 12,
        alignItems: 'center',
        transform: `translate(-50%,-50%) scale(${s})`,
        opacity: out,
        fontFamily: sans,
      }}
    >
      {keys.map((k, i) => (
        <React.Fragment key={k}>
          {i > 0 ? <span style={{color: c.dim, fontSize: 34, fontWeight: 700}}>+</span> : null}
          <div
            style={{
              minWidth: 84,
              height: 84,
              padding: '0 22px',
              borderRadius: 16,
              background: 'linear-gradient(180deg,#3a3c48,#25262f)',
              border: '1px solid rgba(255,255,255,0.18)',
              boxShadow: `0 ${8 - press * 6}px 0 #121218, 0 20px 50px rgba(0,0,0,0.6), 0 0 40px rgba(123,77,255,${0.25 + press * 0.4})`,
              transform: `translateY(${press * 6}px)`,
              display: 'grid',
              placeItems: 'center',
              fontSize: 36,
              fontWeight: 700,
              color: '#fff',
            }}
          >
            {k}
          </div>
        </React.Fragment>
      ))}
    </div>
  );
};

/* ---------- Captions ---------- */

export const SceneHeader: React.FC<{index: string; title: string; sub: string}> = ({index, title, sub}) => {
  const a = useSpring(2);
  const b = useSpring(8);
  const {width, height} = useVideoConfig();
  const vertical = height > width;
  return (
    <div style={{position: 'absolute', left: vertical ? 60 : 80, top: vertical ? 110 : 52, width: width - (vertical ? 120 : 160), fontFamily: sans, color: c.text}}>
      <div style={{display: 'flex', gap: 18, alignItems: 'center', opacity: a, transform: `translateY(${(1 - a) * 20}px)`}}>
        <div
          style={{
            fontFamily: mono,
            fontSize: 20,
            fontWeight: 700,
            padding: '6px 14px',
            borderRadius: 999,
            background: 'rgba(123,77,255,0.18)',
            border: '1px solid rgba(194,145,255,0.4)',
            color: c.violet,
          }}
        >
          {index}
        </div>
        <div style={{fontSize: vertical ? 58 : 46, fontWeight: 800, letterSpacing: -1.2}}>{title}</div>
      </div>
      <div style={{fontSize: vertical ? 30 : 24, lineHeight: 1.35, color: c.dim, marginTop: 10, opacity: b, transform: `translateY(${(1 - b) * 16}px)`}}>{sub}</div>
    </div>
  );
};

export const HeadlessBar: React.FC<{at: number; cmd: string}> = ({at, cmd}) => {
  const frame = useCurrentFrame();
  const {width, height} = useVideoConfig();
  const vertical = height > width;
  const s = useSpring(at, 18);
  const typed = Math.floor(interpolate(frame, [at + 4, at + 4 + cmd.length * 0.6], [0, cmd.length], clamp));
  if (frame < at) return null;
  return (
    <div
      style={{
        position: 'absolute',
        left: '50%',
        bottom: vertical ? 90 : 46,
        transform: `translateX(-50%) translateY(${(1 - s) * 40}px)`,
        opacity: s,
        fontFamily: mono,
        fontSize: vertical ? 21 : 24,
        padding: '14px 26px',
        borderRadius: 12,
        background: 'rgba(17,18,26,0.92)',
        border: `1px solid ${c.line}`,
        boxShadow: '0 20px 60px rgba(0,0,0,0.5)',
        whiteSpace: 'nowrap',
      }}
    >
      <span style={{color: c.muted}}>Headless: </span>
      <span style={{color: c.good}}>{cmd.slice(0, typed)}</span>
      <span style={{opacity: Math.floor(frame / 8) % 2 ? 1 : 0, color: c.good}}>▍</span>
    </div>
  );
};

/* ---------- Context menu ---------- */

export const ContextMenu: React.FC<{x: number; y: number; at: number; items: string[]; highlight: number; highlightAt: number; until: number}> = ({
  x,
  y,
  at,
  items,
  highlight,
  highlightAt,
  until,
}) => {
  const frame = useCurrentFrame();
  const s = useSpring(at, 20, 0.4);
  if (frame < at || frame > until) return null;
  return (
    <div
      style={{
        position: 'absolute',
        left: x,
        top: y,
        width: 280,
        padding: 6,
        borderRadius: 10,
        background: '#2b2c33',
        border: '1px solid rgba(255,255,255,0.14)',
        boxShadow: '0 20px 50px rgba(0,0,0,0.6)',
        fontFamily: sans,
        fontSize: 16,
        color: c.text,
        transformOrigin: 'top left',
        transform: `scale(${0.9 + s * 0.1})`,
        opacity: s,
        zIndex: 5,
      }}
    >
      {items.map((it, i) =>
        it === '-' ? (
          <div key={i} style={{height: 1, background: c.line, margin: '5px 6px'}} />
        ) : (
          <div
            key={i}
            style={{
              padding: '9px 12px',
              borderRadius: 6,
              background: i === highlight && frame >= highlightAt ? 'rgba(104,182,255,0.25)' : 'transparent',
            }}
          >
            {it}
          </div>
        ),
      )}
    </div>
  );
};
