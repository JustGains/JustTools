import React from 'react';
import {interpolate, useCurrentFrame} from 'remotion';
import {c, mono, sans} from './theme';
import {clamp, GitMark} from './ui';

/* ---------- Shared art ---------- */

export const SunsetScene: React.FC<{small?: boolean}> = ({small}) => {
  const f = useCurrentFrame();
  const drift = (f * 1.4) % 200;
  return (
    <div style={{position: 'absolute', inset: 0, background: 'linear-gradient(180deg,#2a1b5c 0%,#b8467a 52%,#ff9d5c 80%)', overflow: 'hidden'}}>
      <div
        style={{
          position: 'absolute',
          left: '50%',
          top: '34%',
          width: small ? 44 : 190,
          height: small ? 44 : 190,
          marginLeft: small ? -22 : -95,
          borderRadius: '50%',
          background: 'repeating-linear-gradient(180deg,#ffe28a 0 12%,transparent 12% 16%)',
          boxShadow: '0 0 80px rgba(255,200,120,0.8)',
        }}
      />
      <svg viewBox="0 0 600 340" preserveAspectRatio="none" style={{position: 'absolute', inset: 0, width: '100%', height: '100%'}}>
        <path d="M0 230 L90 170 L170 215 L260 150 L360 220 L450 160 L600 225 L600 340 L0 340Z" fill="#3b1d4f" />
        <path d="M0 260 L120 215 L230 250 L340 205 L470 255 L600 230 L600 340 L0 340Z" fill="#241238" />
        <path d="M240 340 L292 250 L308 250 L360 340Z" fill="#14091f" />
        {[0, 1, 2, 3].map((i) => {
          const t = ((drift + i * 50) % 200) / 200;
          return <rect key={i} x={298 - t * 8} y={252 + t * 88} width={4 + t * 16} height={4 + t * 18} fill="#ffd27a" opacity={0.9} />;
        })}
      </svg>
    </div>
  );
};

export const PartyCat: React.FC<{small?: boolean}> = ({small}) => {
  const f = useCurrentFrame();
  const bob = Math.sin(f / 3.2) * (small ? 4 : 12);
  const tilt = Math.sin(f / 4.5) * 10;
  const hue = (f * 6) % 360;
  return (
    <div
      style={{
        position: 'absolute',
        inset: 0,
        overflow: 'hidden',
        background: `conic-gradient(from ${f * 4}deg at 50% 60%, #ff5f6d, #ffc371, #6fd697, #68b6ff, #c291ff, #ff5f6d)`,
        filter: `hue-rotate(${hue / 6}deg)`,
      }}
    >
      <svg viewBox="0 0 200 160" style={{position: 'absolute', inset: 0, width: '100%', height: '100%'}}>
        <g transform={`translate(100 ${92 + bob}) rotate(${tilt})`}>
          <path d="M-50 -18 L-42 -62 L-14 -36 Z M50 -18 L42 -62 L14 -36 Z" fill="#2b2b33" />
          <ellipse cx="0" cy="0" rx="56" ry="46" fill="#2b2b33" />
          <path d="M-22 -60 L0 -98 L22 -60 Z" fill="#ffe14d" stroke="#ff5f6d" strokeWidth="3" />
          <circle cx="0" cy="-100" r="6" fill="#ff5f6d" />
          <ellipse cx="-20" cy="-4" rx="9" ry={f % 40 < 3 ? 1.5 : 10} fill="#9dff9d" />
          <ellipse cx="20" cy="-4" rx="9" ry={f % 40 < 3 ? 1.5 : 10} fill="#9dff9d" />
          <path d="M-6 14 Q0 20 6 14" stroke="#ff9fb2" strokeWidth="3" fill="none" />
          <path d="M-30 12 L-62 6 M-30 18 L-62 22 M30 12 L62 6 M30 18 L62 22" stroke="#cfcfd8" strokeWidth="2" />
        </g>
      </svg>
    </div>
  );
};

export const Avatar: React.FC<{round?: boolean}> = ({round}) => (
  <div
    style={{
      position: 'absolute',
      inset: 0,
      borderRadius: round ? '50%' : 0,
      overflow: 'hidden',
      background: 'radial-gradient(circle at 30% 25%, #ffd1a6, #ff7aa8 45%, #6a5cff 100%)',
    }}
  >
    <svg viewBox="0 0 100 100" style={{position: 'absolute', inset: 0, width: '100%', height: '100%'}}>
      <circle cx="50" cy="40" r="19" fill="#20152f" />
      <path d="M31 38 Q33 16 52 17 Q71 18 70 42 Q66 28 50 27 Q37 28 31 38Z" fill="#100a1a" />
      <path d="M14 100 Q16 66 50 64 Q84 66 86 100Z" fill="#20152f" />
      <circle cx="43" cy="41" r="2.2" fill="#ffd1a6" />
      <circle cx="57" cy="41" r="2.2" fill="#ffd1a6" />
      <path d="M44 49 Q50 53 56 49" stroke="#ffd1a6" strokeWidth="2" fill="none" strokeLinecap="round" />
    </svg>
  </div>
);

/* ---------- YouTube-style watch page ---------- */

export const YouTubeIcon = (
  <div style={{width: 18, height: 13, borderRadius: 4, background: '#ff0033', display: 'grid', placeItems: 'center'}}>
    <div style={{width: 0, height: 0, borderLeft: '6px solid #fff', borderTop: '3.5px solid transparent', borderBottom: '3.5px solid transparent', marginLeft: 1}} />
  </div>
);

export const VideoPage: React.FC = () => {
  const f = useCurrentFrame();
  return (
    <div style={{position: 'absolute', inset: 0, background: '#0f0f0f', fontFamily: sans, color: '#f1f1f1'}}>
      <div style={{height: 54, display: 'flex', alignItems: 'center', padding: '0 22px', gap: 10}}>
        <span style={{fontSize: 20, color: c.dim}}>☰</span>
        <div style={{transform: 'scale(1.5)', marginLeft: 12}}>{YouTubeIcon}</div>
        <span style={{fontWeight: 700, fontSize: 19, letterSpacing: -0.6, marginLeft: 4}}>YouTube</span>
        <div style={{marginLeft: 70, width: 380, height: 36, borderRadius: 18, border: '1px solid #303030', background: '#121212', color: '#888', fontSize: 15, display: 'flex', alignItems: 'center', padding: '0 16px'}}>
          Search
        </div>
      </div>
      <div style={{display: 'flex', gap: 22, padding: '6px 22px'}}>
        <div style={{width: 620}}>
          <div style={{width: 620, height: 349, borderRadius: 12, position: 'relative', overflow: 'hidden'}}>
            <SunsetScene />
            <div style={{position: 'absolute', left: 0, right: 0, bottom: 0, height: 44, background: 'linear-gradient(transparent,rgba(0,0,0,0.7))'}} />
            <div style={{position: 'absolute', left: 14, right: 14, bottom: 30, height: 4, background: 'rgba(255,255,255,0.3)', borderRadius: 2}}>
              <div style={{width: `${18 + f * 0.05}%`, height: '100%', background: '#ff0033', borderRadius: 2}} />
            </div>
            <div style={{position: 'absolute', left: 16, bottom: 8, fontSize: 13, fontFamily: mono}}>▶  1:42 / 9:58</div>
            <div style={{position: 'absolute', right: 16, bottom: 8, fontSize: 13, fontWeight: 700, padding: '0 5px', borderRadius: 3, background: 'rgba(255,255,255,0.18)'}}>4K</div>
          </div>
          <div style={{fontSize: 22, fontWeight: 700, marginTop: 14}}>Sunset Drive — 4K Lofi Ride Through the Desert</div>
          <div style={{display: 'flex', alignItems: 'center', gap: 12, marginTop: 14}}>
            <div style={{width: 40, height: 40, borderRadius: 20, background: 'linear-gradient(135deg,#ff9d5c,#b8467a)'}} />
            <div>
              <div style={{fontSize: 15, fontWeight: 600}}>Golden Hour Tapes</div>
              <div style={{fontSize: 13, color: '#aaa'}}>412K subscribers</div>
            </div>
            <div style={{marginLeft: 16, padding: '9px 16px', borderRadius: 18, background: '#f1f1f1', color: '#0f0f0f', fontSize: 14, fontWeight: 600}}>Subscribe</div>
          </div>
        </div>
        <div style={{flex: 1, display: 'flex', flexDirection: 'column', gap: 12}}>
          {['#3a2a6a,#ff7a59', '#0f4c5c,#5fd3c4', '#4b1d3f,#ffb1c8', '#1c2a4a,#68b6ff', '#3d2b1f,#ffd27a'].map((g, i) => (
            <div key={i} style={{display: 'flex', gap: 8}}>
              <div style={{width: 128, height: 72, borderRadius: 8, background: `linear-gradient(135deg,${g})`}} />
              <div style={{flex: 1, display: 'flex', flexDirection: 'column', gap: 6, paddingTop: 4}}>
                <div style={{height: 10, borderRadius: 5, background: '#2a2a2a', width: '95%'}} />
                <div style={{height: 10, borderRadius: 5, background: '#2a2a2a', width: '70%'}} />
                <div style={{height: 8, borderRadius: 4, background: '#1f1f1f', width: '50%'}} />
              </div>
            </div>
          ))}
        </div>
      </div>
    </div>
  );
};

/* ---------- GIF gallery ---------- */

export const GifPage: React.FC = () => (
  <div style={{position: 'absolute', inset: 0, background: '#16151d', fontFamily: sans, color: c.text}}>
    <div style={{height: 56, display: 'flex', alignItems: 'center', padding: '0 24px', gap: 14, borderBottom: `1px solid ${c.line}`}}>
      <div style={{fontWeight: 800, fontSize: 22, background: 'linear-gradient(90deg,#ff5f6d,#ffc371)', WebkitBackgroundClip: 'text', color: 'transparent'}}>loopz</div>
      <span style={{color: c.dim, fontSize: 15}}>Trending · Reactions · Cats · Party</span>
    </div>
    <div style={{display: 'grid', gridTemplateColumns: '1.6fr 1fr 1fr', gridTemplateRows: '230px 230px', gap: 14, padding: 22}}>
      <div style={{gridRow: 'span 2', borderRadius: 14, overflow: 'hidden', position: 'relative'}}>
        <PartyCat />
        <div style={{position: 'absolute', left: 12, top: 12, fontSize: 12, fontWeight: 800, padding: '3px 8px', borderRadius: 5, background: 'rgba(0,0,0,0.55)'}}>GIF</div>
      </div>
      {['#2b5876,#4e4376', '#ff9a9e,#fad0c4', '#a18cd1,#fbc2eb', '#43e97b,#38f9d7'].map((g) => (
        <div key={g} style={{borderRadius: 14, background: `linear-gradient(135deg,${g})`, opacity: 0.85}} />
      ))}
    </div>
    <div style={{padding: '0 24px', fontSize: 18, fontWeight: 600}}>party cat 🎉</div>
    <div style={{padding: '4px 24px', fontSize: 14, color: c.dim}}>loopz.gg/gifs/party-cat · 482 × 360 · 2.4 MB</div>
  </div>
);

/* ---------- GitHub-style repository page ---------- */

const repoRows: [string, boolean, string, string][] = [
  ['.github', true, 'build: Improve SHA256 checksum generation', '3 weeks ago'],
  ['docs', true, 'feat: release JustTools 2.7 with portable clipboard paste', '2 days ago'],
  ['explorer', true, 'Windows context menu, JustPaste, video resolutions', 'last week'],
  ['skills/justtools', true, 'feat: release JustTools 2.7', '2 days ago'],
  ['src', true, 'feat: release JustTools 2.7 with Git cloning', '2 days ago'],
  ['AGENTS.md', false, 'feat: release JustTools 2.7', '2 days ago'],
  ['Cargo.toml', false, 'feat: release JustTools 2.7', '2 days ago'],
  ['README.md', false, 'feat: release JustTools 2.7', '2 days ago'],
];

export const RepoPage: React.FC = () => (
  <div style={{position: 'absolute', inset: 0, background: '#0d1117', fontFamily: sans, color: '#e6edf3'}}>
    <div style={{height: 60, display: 'flex', alignItems: 'center', padding: '0 24px', gap: 12, background: '#010409', borderBottom: '1px solid #30363d'}}>
      <div style={{width: 32, height: 32, borderRadius: 16, background: '#e6edf3', display: 'grid', placeItems: 'center'}}>
        <GitMark size={18} color="#0d1117" />
      </div>
      <span style={{fontSize: 17}}>
        JustGains <span style={{color: '#7d8590'}}>/</span> <b>JustTools</b>
      </span>
      <span style={{fontSize: 12, border: '1px solid #30363d', borderRadius: 999, padding: '2px 8px', color: '#7d8590'}}>Public</span>
    </div>
    <div style={{display: 'flex', gap: 24, padding: '0 24px', height: 44, alignItems: 'center', fontSize: 14, color: '#7d8590', borderBottom: '1px solid #30363d'}}>
      <span style={{color: '#e6edf3', borderBottom: '2px solid #f78166', paddingBottom: 10, marginTop: 12}}>‹› Code</span>
      <span>Issues</span>
      <span>Pull requests</span>
      <span>Actions</span>
      <span>Releases</span>
    </div>
    <div style={{display: 'flex', gap: 20, padding: 22}}>
      <div style={{flex: 1}}>
        <div style={{display: 'flex', alignItems: 'center', gap: 10, marginBottom: 14}}>
          <span style={{fontSize: 14, padding: '6px 12px', border: '1px solid #30363d', borderRadius: 6, background: '#21262d'}}>⑂ main ▾</span>
          <div style={{flex: 1}} />
          <span style={{fontSize: 14, padding: '6px 14px', borderRadius: 6, background: '#238636', fontWeight: 600}}>‹› Code ▾</span>
        </div>
        <div style={{border: '1px solid #30363d', borderRadius: 8, overflow: 'hidden'}}>
          <div style={{padding: '10px 14px', background: '#161b22', fontSize: 13, color: '#7d8590'}}>
            <b style={{color: '#e6edf3'}}>JustGains</b> feat: release JustTools 2.7 with portable clipboard paste and Git cloning
          </div>
          {repoRows.map(([name, dir, msg, when]) => (
            <div key={name} style={{display: 'flex', gap: 10, padding: '8px 14px', borderTop: '1px solid #21262d', fontSize: 13.5, alignItems: 'center'}}>
              <span style={{width: 16, color: dir ? '#7d8590' : '#7d8590'}}>{dir ? '▰' : '▭'}</span>
              <span style={{width: 150}}>{name}</span>
              <span style={{flex: 1, color: '#7d8590', whiteSpace: 'nowrap', overflow: 'hidden', textOverflow: 'ellipsis'}}>{msg}</span>
              <span style={{color: '#7d8590'}}>{when}</span>
            </div>
          ))}
        </div>
      </div>
      <div style={{width: 210, fontSize: 14, color: '#7d8590', lineHeight: 1.5}}>
        <div style={{color: '#e6edf3', fontWeight: 600, marginBottom: 8}}>About</div>
        One portable Rust binary of small, safe tools for images, media, Git, and the clipboard.
        <div style={{marginTop: 14}}>☆ Star · ⑂ Fork</div>
        <div style={{marginTop: 8}}>◉ Rust 96.1%</div>
      </div>
    </div>
  </div>
);

/* ---------- X profile ---------- */

export const XIcon = <span style={{fontWeight: 900, fontSize: 15, fontFamily: sans}}>𝕏</span>;

export const ProfilePage: React.FC = () => (
  <div style={{position: 'absolute', inset: 0, background: '#000', fontFamily: sans, color: '#e7e9ea', display: 'flex'}}>
    <div style={{width: 90, borderRight: '1px solid #2f3336', display: 'flex', flexDirection: 'column', alignItems: 'center', paddingTop: 20, gap: 30, fontSize: 24, color: '#e7e9ea'}}>
      <span style={{fontSize: 30, fontWeight: 900}}>𝕏</span>
      <span>⌂</span>
      <span>⌕</span>
      <span>🔔</span>
      <span>✉</span>
    </div>
    <div style={{width: 600, borderRight: '1px solid #2f3336', position: 'relative'}}>
      <div style={{height: 54, padding: '6px 18px'}}>
        <div style={{fontSize: 19, fontWeight: 800}}>Nova Park</div>
        <div style={{fontSize: 13, color: '#71767b'}}>3,881 posts</div>
      </div>
      <div style={{height: 170, background: 'linear-gradient(120deg,#2b1a6b,#7b4dff 40%,#4f7dff 70%,#5fd3c4)'}} />
      <div style={{position: 'absolute', left: 18, top: 160, width: 128, height: 128, borderRadius: '50%', border: '4px solid #000'}}>
        <div style={{position: 'absolute', inset: 0, borderRadius: '50%', overflow: 'hidden'}}>
          <Avatar round />
        </div>
      </div>
      <div style={{display: 'flex', justifyContent: 'flex-end', padding: '12px 18px'}}>
        <span style={{padding: '8px 18px', borderRadius: 999, background: '#e7e9ea', color: '#0f1419', fontWeight: 700, fontSize: 15}}>Follow</span>
      </div>
      <div style={{padding: '22px 18px 0'}}>
        <div style={{fontSize: 22, fontWeight: 800}}>Nova Park</div>
        <div style={{fontSize: 15, color: '#71767b'}}>@novapark</div>
        <div style={{fontSize: 15, marginTop: 12}}>Designing tiny tools for big days ✦ type nerd ✦ Brooklyn</div>
        <div style={{fontSize: 14, color: '#71767b', marginTop: 10}}>📅 Joined March 2019</div>
        <div style={{fontSize: 14, marginTop: 10}}>
          <b>1,204</b> <span style={{color: '#71767b'}}>Following</span> &nbsp; <b>48.2K</b> <span style={{color: '#71767b'}}>Followers</span>
        </div>
      </div>
      <div style={{display: 'flex', marginTop: 18, borderBottom: '1px solid #2f3336', fontSize: 15, color: '#71767b'}}>
        {['Posts', 'Replies', 'Highlights', 'Media'].map((t, i) => (
          <div key={t} style={{flex: 1, textAlign: 'center', padding: '14px 0', color: i === 0 ? '#e7e9ea' : undefined, fontWeight: i === 0 ? 700 : 400}}>
            {t}
          </div>
        ))}
      </div>
    </div>
    <div style={{flex: 1, padding: 16}}>
      <div style={{height: 38, borderRadius: 19, background: '#202327', color: '#71767b', fontSize: 14, display: 'flex', alignItems: 'center', padding: '0 14px'}}>Search</div>
    </div>
  </div>
);

/* ---------- Article ---------- */

export const ArticleIcon = <div style={{width: 16, height: 16, borderRadius: 4, background: 'linear-gradient(135deg,#f38020,#faae40)'}} />;

export const ArticlePage: React.FC = () => (
  <div style={{position: 'absolute', inset: 0, background: '#fbfaf7', fontFamily: sans, color: '#1b1b1f'}}>
    <div style={{height: 54, display: 'flex', alignItems: 'center', padding: '0 40px', gap: 12, borderBottom: '1px solid #e8e5dd'}}>
      {ArticleIcon}
      <b style={{fontSize: 17}}>Edgewise</b>
      <span style={{color: '#8a877e', fontSize: 14}}>Engineering blog</span>
      <div style={{flex: 1}} />
      <span style={{fontSize: 14, color: '#8a877e'}}>Subscribe</span>
    </div>
    <div style={{padding: '34px 110px'}}>
      <div style={{fontSize: 13, letterSpacing: 2, color: '#f38020', fontWeight: 700}}>PERFORMANCE</div>
      <div style={{fontSize: 40, fontWeight: 800, letterSpacing: -1.2, lineHeight: 1.1, marginTop: 10}}>Shaving cold starts to zero</div>
      <div style={{fontSize: 15, color: '#8a877e', marginTop: 12}}>By Ada Moreno · 7 min read</div>
      <p style={{fontSize: 17, lineHeight: 1.6, marginTop: 20, color: '#3a3a40'}}>
        Every millisecond before the first byte is a millisecond your user feels. Here is how we moved isolates ahead of the request and made
        cold starts disappear.
      </p>
      <div style={{fontSize: 22, fontWeight: 700, marginTop: 18}}>Warm before you're asked</div>
      <div style={{marginTop: 12, borderRadius: 8, background: '#1e1e24', color: '#e6e6ea', fontFamily: mono, fontSize: 14, padding: '14px 18px', lineHeight: 1.6}}>
        <span style={{color: '#c291ff'}}>export default</span> {'{'}
        <br />
        &nbsp;&nbsp;<span style={{color: '#68b6ff'}}>async</span> fetch(req, env) {'{'} <span style={{color: '#8a93a6'}}>// pre-warmed</span>
      </div>
    </div>
  </div>
);
