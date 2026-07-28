import { Composition } from "remotion";
import { FlagshipFilm } from "./FlagshipFilm";
import { ProductCut } from "./ProductCut";

export const FPS = 30;
export const DURATION = 42 * FPS;

export function Root() {
  return (
    <>
      <Composition
        id="ClarkCodeFlagship"
        component={FlagshipFilm}
        durationInFrames={DURATION}
        fps={FPS}
        width={1920}
        height={1080}
      />
      <Composition
        id="ClarkCodeProductCut"
        component={ProductCut}
        durationInFrames={30 * FPS}
        fps={FPS}
        width={1920}
        height={1080}
      />
    </>
  );
}
