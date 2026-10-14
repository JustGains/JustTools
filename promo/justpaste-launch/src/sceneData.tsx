import React from 'react';
import {ArticleIcon, ArticlePage, Avatar, GifPage, PartyCat, ProfilePage, RepoPage, SunsetScene, VideoPage, XIcon, YouTubeIcon} from './pages';
import {MarkdownPreview} from './scenes';
import {GitMark} from './ui';
import {UseCaseProps} from './UseCase';

const PlayBadge = () => (
  <div style={{position: 'absolute', inset: 0, display: 'grid', placeItems: 'center'}}>
    <div style={{width: 34, height: 34, borderRadius: 17, background: 'rgba(0,0,0,0.55)', display: 'grid', placeItems: 'center'}}>
      <div style={{width: 0, height: 0, borderLeft: '11px solid #fff', borderTop: '7px solid transparent', borderBottom: '7px solid transparent', marginLeft: 3}} />
    </div>
  </div>
);

export type Scene = UseCaseProps & {duration: number};

export const scenes: Scene[] = [
  {
    duration: 260,
    index: '01',
    title: 'A video from YouTube',
    sub: 'Copy the link, Ctrl+V in Explorer. Highest resolution, MP4, named after the video.',
    url: 'youtube.com/watch?v=8fJ2kLmQ0aE',
    tab: 'Sunset Drive — 4K Lofi Ride',
    tabIcon: YouTubeIcon,
    page: <VideoPage />,
    copy: {mode: 'address'},
    folder: 'Videos',
    files: [
      {name: 'b-roll', kind: 'folder'},
      {name: 'intro-v2.mp4', kind: 'video', meta: '86.1 MB'},
    ],
    result: {
      name: 'Sunset Drive 4K Lofi Ride Through the Desert.mp4',
      kind: 'video',
      meta: '248.6 MB',
      thumb: (
        <>
          <SunsetScene small />
          <PlayBadge />
        </>
      ),
    },
    steps: [
      {at: 124, text: 'Preparing your paste…'},
      {at: 140, text: 'Downloading media…', detail: '21.4 MB/s · 2160p', pct: [0, 100]},
      {at: 192, text: 'Merging audio and video…'},
    ],
    resultAt: 208,
    toast: {title: 'Saved to Videos', body: 'Sunset Drive 4K Lofi Ride Through… · 248.6 MB'},
    cmd: 'justpaste https://www.youtube.com/watch?v=8fJ2kLmQ0aE',
    hue: 0,
  },
  {
    duration: 190,
    index: '02',
    title: 'A GIF from any site',
    sub: 'Copy image address and paste. The original file, byte for byte — animation intact.',
    url: 'loopz.gg/gifs/party-cat',
    tab: 'party cat 🎉 — loopz',
    tabIcon: <div style={{width: 16, height: 16, borderRadius: 4, overflow: 'hidden', position: 'relative'}}><PartyCat small /></div>,
    page: <GifPage />,
    copy: {
      mode: 'menu',
      x: 220,
      y: 344,
      items: ['Open image in new tab', 'Save image as…', 'Copy image', 'Copy image address', '-', 'Inspect'],
      highlight: 3,
    },
    folder: 'Moodboard',
    files: [
      {name: 'refs', kind: 'folder'},
      {name: 'palette.png', kind: 'image', meta: '312 KB'},
    ],
    result: {name: 'party-cat.gif', kind: 'gif', meta: '2.4 MB', thumb: <PartyCat small />},
    steps: [],
    resultAt: 124,
    toast: {title: 'Saved to Moodboard', body: 'party-cat.gif · 2.4 MB'},
    cmd: 'justpaste https://media.loopz.gg/party-cat.gif',
    hue: 25,
  },
  {
    duration: 240,
    index: '03',
    title: 'A Git repository',
    sub: 'Paste a repo link into any folder and it is cloned — history, branches and origin.',
    url: 'github.com/JustGains/JustTools',
    tab: 'JustGains/JustTools',
    tabIcon: <GitMark size={16} />,
    page: <RepoPage />,
    copy: {mode: 'address'},
    folder: 'Code',
    files: [
      {name: 'dotfiles', kind: 'folder'},
      {name: 'notes.txt', kind: 'txt', meta: '2 KB'},
    ],
    result: {name: 'JustTools', kind: 'repo', meta: 'Git repository'},
    steps: [
      {at: 124, text: 'Preparing your paste…'},
      {at: 136, text: 'Cloning JustTools…', detail: 'Receiving objects', pct: [0, 100]},
    ],
    resultAt: 186,
    toast: {title: 'Cloned into Code', body: 'JustTools · default branch checked out'},
    cmd: 'justpaste https://github.com/JustGains/JustTools',
    hue: -15,
  },
  {
    duration: 230,
    index: '04',
    title: 'A profile picture from X',
    sub: 'Paste an x.com profile link. Get the full-size avatar, not a 48-pixel crop.',
    url: 'x.com/novapark',
    tab: 'Nova Park (@novapark) / X',
    tabIcon: XIcon,
    page: <ProfilePage />,
    copy: {mode: 'address'},
    folder: 'Brand',
    files: [
      {name: 'logos', kind: 'folder'},
      {name: 'press-kit.zip', kind: 'zip', meta: '18.4 MB'},
    ],
    result: {name: 'novapark.jpg', kind: 'image', meta: '1080 × 1080', thumb: <Avatar />},
    steps: [
      {at: 124, text: 'Finding the profile picture…'},
      {at: 150, text: 'Downloading novapark.jpg', detail: 'full size', pct: [0, 100]},
    ],
    resultAt: 176,
    toast: {title: 'Saved to Brand', body: 'novapark.jpg · 1080 × 1080'},
    cmd: 'justpaste https://x.com/novapark',
    hue: 40,
  },
  {
    duration: 320,
    index: '05',
    title: 'Any page as Markdown',
    sub: "Paste an article and get clean Markdown, rendered by Cloudflare's /markdown endpoint.",
    url: 'edgewise.dev/blog/cold-starts',
    tab: 'Shaving cold starts to zero',
    tabIcon: ArticleIcon,
    page: <ArticlePage />,
    copy: {mode: 'address'},
    folder: 'Research',
    files: [
      {name: 'papers', kind: 'folder'},
      {name: 'Q3 notes.md', kind: 'md', meta: '6 KB'},
    ],
    result: {name: 'Shaving cold starts to zero.md', kind: 'md', meta: '14.2 KB'},
    steps: [
      {at: 124, text: 'Preparing your paste…'},
      {at: 134, text: 'Rendering page with Cloudflare Browser Run…'},
      {at: 166, text: 'Converting to Markdown…', pct: [0, 100]},
    ],
    resultAt: 188,
    toast: {title: 'Saved to Research', body: 'Shaving cold starts to zero.md · 14.2 KB'},
    cmd: 'justpaste --markdown https://edgewise.dev/blog/cold-starts',
    extra: <MarkdownPreview at={210} />,
    hue: -30,
  },
];

export const INTRO = 150;
export const EVERYWHERE = 210;
export const OUTRO = 180;
export const T = 16;

export const TOTAL = INTRO + scenes.reduce((n, s) => n + s.duration, 0) + EVERYWHERE + OUTRO - T * (scenes.length + 2);
