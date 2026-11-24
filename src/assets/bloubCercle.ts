import attentif from "./bloub-cercle/bloub-cercle-attentif-encre-anime.svg";
import blase from "./bloub-cercle/bloub-cercle-blase-encre-anime.svg";
import colere from "./bloub-cercle/bloub-cercle-colere-encre-anime.svg";
import confus from "./bloub-cercle/bloub-cercle-confus-encre-anime.svg";
import curieux from "./bloub-cercle/bloub-cercle-curieux-encre-anime.svg";
import effraye from "./bloub-cercle/bloub-cercle-effraye-encre-anime.svg";
import excite from "./bloub-cercle/bloub-cercle-excite-encre-anime.svg";
import fier from "./bloub-cercle/bloub-cercle-fier-encre-anime.svg";
import heureux from "./bloub-cercle/bloub-cercle-heureux-encre-anime.svg";
import hilare from "./bloub-cercle/bloub-cercle-hilare-encre-anime.svg";
import mefiant from "./bloub-cercle/bloub-cercle-mefiant-encre-anime.svg";
import neutre from "./bloub-cercle/bloub-cercle-neutre-encre-anime.svg";
import pensif from "./bloub-cercle/bloub-cercle-pensif-encre-anime.svg";
import somnolent from "./bloub-cercle/bloub-cercle-somnolent-encre-anime.svg";
import surpris from "./bloub-cercle/bloub-cercle-surpris-encre-anime.svg";
import timide from "./bloub-cercle/bloub-cercle-timide-encre-anime.svg";
import triste from "./bloub-cercle/bloub-cercle-triste-encre-anime.svg";

export const BLOUB_CERCLE_SVGS = {
  attentif,
  blase,
  colere,
  confus,
  curieux,
  effraye,
  excite,
  fier,
  heureux,
  hilare,
  mefiant,
  neutre,
  pensif,
  somnolent,
  surpris,
  timide,
  triste,
} as const;

export type BloubEmotion = keyof typeof BLOUB_CERCLE_SVGS;

export const DEFAULT_BLOUB_EMOTION: BloubEmotion = "neutre";
