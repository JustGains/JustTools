import React from 'react';
import {Composition} from 'remotion';
import {linearTiming, TransitionSeries} from '@remotion/transitions';
import {fade} from '@remotion/transitions/fade';
import {slide} from '@remotion/transitions/slide';
import {Everywhere, Intro, Outro} from './scenes';
import {EVERYWHERE, INTRO, OUTRO, scenes, T, TOTAL} from './sceneData';
import {FPS, HEIGHT, WIDTH} from './theme';
import {UseCase} from './UseCase';
import {Launch98} from './win98/scenes98';


const Launch: React.FC = () => (
  <TransitionSeries>
    <TransitionSeries.Sequence durationInFrames={INTRO}>
      <Intro />
    </TransitionSeries.Sequence>
    {scenes.map(({duration, ...props}, i) => (
      <React.Fragment key={props.index}>
        <TransitionSeries.Transition
          presentation={i === 0 ? fade() : slide({direction: 'from-right'})}
          timing={linearTiming({durationInFrames: T})}
        />
        <TransitionSeries.Sequence durationInFrames={duration}>
          <UseCase {...props} />
        </TransitionSeries.Sequence>
      </React.Fragment>
    ))}
    <TransitionSeries.Transition presentation={slide({direction: 'from-bottom'})} timing={linearTiming({durationInFrames: T})} />
    <TransitionSeries.Sequence durationInFrames={EVERYWHERE}>
      <Everywhere />
    </TransitionSeries.Sequence>
    <TransitionSeries.Transition presentation={fade()} timing={linearTiming({durationInFrames: T})} />
    <TransitionSeries.Sequence durationInFrames={OUTRO}>
      <Outro />
    </TransitionSeries.Sequence>
  </TransitionSeries>
);

export const Root: React.FC = () => (
  <>
    <Composition id="JustPasteLaunch" component={Launch} durationInFrames={TOTAL} fps={FPS} width={WIDTH} height={HEIGHT} />
    <Composition id="JustPasteReel" component={Launch} durationInFrames={TOTAL} fps={FPS} width={HEIGHT} height={WIDTH} />
    <Composition id="JustPaste98Launch" component={Launch98} durationInFrames={TOTAL} fps={FPS} width={WIDTH} height={HEIGHT} />
    <Composition id="JustPaste98Reel" component={Launch98} durationInFrames={TOTAL} fps={FPS} width={HEIGHT} height={WIDTH} />
  </>
);

