import {loadFont as loadInter} from '@remotion/google-fonts/Inter';
import {loadFont as loadMono} from '@remotion/google-fonts/JetBrainsMono';

export const sans = loadInter('normal', {weights: ['400', '500', '600', '700', '800'], subsets: ['latin']}).fontFamily;
export const mono = loadMono('normal', {weights: ['400', '500', '700'], subsets: ['latin']}).fontFamily;

// Mirrors src/console_ui.rs and the violet-and-blue tray icon.
export const c = {
  bg: '#07070d',
  panel: '#11121a',
  panel2: '#181a24',
  line: 'rgba(255,255,255,0.08)',
  text: '#eef0f6',
  dim: '#8a93a6',
  muted: '#767f91',
  good: '#6fd697',
  accent: '#68b6ff',
  violet: '#c291ff',
  brand: '#7b4dff',
  brand2: '#4f7dff',
  warning: '#f5b85a',
  selected: '#243040',
};

export const brandGradient = `linear-gradient(135deg, ${c.brand} 0%, ${c.brand2} 100%)`;

export const FPS = 30;
export const WIDTH = 1920;
export const HEIGHT = 1080;
