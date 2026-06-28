import type { Transition, Variants, TargetAndTransition } from "framer-motion";

import { getMotionTokens, type AnimationMode } from "./config";

type Axis = "x" | "y";

function axisOffset(axis: Axis, value: number): { x: number } | { y: number } {
  if (axis === "x") {
    return { x: value };
  }

  return { y: value };
}

function tween(duration: number): Transition {
  return {
    type: "tween",
    duration,
    ease: [0.2, 0, 0, 1] as const,
  };
}

// Accelerate curve for exits - elements leave quickly (ease-in, "fast out")
function tweenExit(duration: number): Transition {
  return {
    type: "tween",
    duration,
    ease: [0.4, 0, 1, 1] as const,
  };
}

export type MotionVariants = Variants & {
  initial: TargetAndTransition;
  animate: TargetAndTransition;
  exit: TargetAndTransition;
};

export function createFadeSlideVariants(mode: AnimationMode, axis: Axis = "y"): MotionVariants {
  const tokens = getMotionTokens(mode);

  return {
    initial: {
      opacity: 0,
      ...axisOffset(axis, tokens.distance.sm),
    },
    animate: {
      opacity: 1,
      ...axisOffset(axis, 0),
      transition: tween(tokens.duration.normal),
    },
    exit: {
      opacity: 0,
      ...axisOffset(axis, tokens.distance.xs),
      transition: tweenExit(tokens.duration.fast),
    },
  };
}

export function createScaleFadeVariants(mode: AnimationMode): MotionVariants {
  const tokens = getMotionTokens(mode);

  return {
    initial: {
      opacity: 0,
      scale: 0.98,
    },
    animate: {
      opacity: 1,
      scale: 1,
      transition: tween(tokens.duration.normal),
    },
    exit: {
      opacity: 0,
      scale: 0.99,
      transition: tweenExit(tokens.duration.fast),
    },
  };
}

/** Directional slide for calendar-style page navigation. direction: 1=forward, -1=backward. */
export function createDirectionalSlideVariants(mode: AnimationMode, direction: 1 | -1): MotionVariants {
  const tokens = getMotionTokens(mode);
  const slideIn = mode === "off" ? 0 : mode === "subtle" ? 30 : mode === "medium" ? 50 : 70;
  const slideOut = mode === "off" ? 0 : mode === "subtle" ? 20 : mode === "medium" ? 35 : 50;

  return {
    initial: { opacity: 0, x: direction > 0 ? slideIn : -slideIn },
    animate: { opacity: 1, x: 0, transition: tween(tokens.duration.normal) },
    exit: { opacity: 0, x: direction > 0 ? -slideOut : slideOut, transition: tweenExit(tokens.duration.fast) },
  };
}

export function createStaggerTransition(mode: AnimationMode): Transition {
  const tokens = getMotionTokens(mode);
  return {
    staggerChildren: tokens.stagger.list,
    delayChildren: 0,
  };
}
