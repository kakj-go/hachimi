import type {
  InteractionRegion,
  MotionCatalogEntry,
  MotionCategory,
  MotionPlaybackMode,
  MotionRootMode,
} from "@hachimi/contracts";
import type { AppLocale } from "@hachimi/i18n";

export const MOTION_CATEGORIES: readonly MotionCategory[] = [
  "idle",
  "reaction",
  "gesture",
  "speech",
  "locomotion",
  "performance",
];

export const INTERACTION_REGIONS: readonly InteractionRegion[] = [
  "head_top",
  "face",
  "chest",
  "belly",
  "hips",
  "left_hand",
  "right_hand",
  "left_arm",
  "right_arm",
  "left_leg",
  "right_leg",
  "foot",
  "generic",
];

const categoryLabels: Record<MotionCategory, readonly [string, string]> = {
  idle: ["待机", "Idle"],
  reaction: ["互动反应", "Reaction"],
  gesture: ["手势", "Gesture"],
  speech: ["说话动作", "Speaking"],
  locomotion: ["移动", "Locomotion"],
  performance: ["表演", "Performance"],
};

const playbackLabels: Record<MotionPlaybackMode, readonly [string, string]> = {
  once: ["单次", "Once"],
  loop: ["循环", "Loop"],
  hold: ["保持", "Hold"],
};

const rootLabels: Record<MotionRootMode, readonly [string, string]> = {
  discard: ["忽略位移", "Discard root"],
  in_place: ["原地", "In place"],
  stage: ["舞台移动", "Stage"],
};

const regionLabels: Record<InteractionRegion, readonly [string, string]> = {
  head_top: ["头顶", "Head top"],
  face: ["脸部", "Face"],
  chest: ["胸部", "Chest"],
  belly: ["腹部", "Belly"],
  hips: ["胯部", "Hips"],
  left_hand: ["左手", "Left hand"],
  right_hand: ["右手", "Right hand"],
  left_arm: ["左手臂", "Left arm"],
  right_arm: ["右手臂", "Right arm"],
  left_leg: ["左腿", "Left leg"],
  right_leg: ["右腿", "Right leg"],
  foot: ["足部", "Foot"],
  generic: ["通用区域", "Generic"],
};

function localized(pair: readonly [string, string], locale: AppLocale): string {
  return locale === "zh-CN" ? pair[0] : pair[1];
}

export function motionName(entry: MotionCatalogEntry, locale: AppLocale): string {
  return locale === "zh-CN" ? entry.nameZh?.trim() || entry.name : entry.name;
}

export function motionDescription(entry: MotionCatalogEntry, locale: AppLocale): string {
  return locale === "zh-CN" ? entry.descriptionZh?.trim() || entry.description : entry.description;
}

export function motionCategoryLabel(value: MotionCategory, locale: AppLocale): string {
  return localized(categoryLabels[value], locale);
}

export function motionPlaybackLabel(value: MotionPlaybackMode, locale: AppLocale): string {
  return localized(playbackLabels[value], locale);
}

export function motionRootLabel(value: MotionRootMode, locale: AppLocale): string {
  return localized(rootLabels[value], locale);
}

export function interactionRegionLabel(value: InteractionRegion, locale: AppLocale): string {
  return localized(regionLabels[value], locale);
}
