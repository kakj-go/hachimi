import type {
  InteractionRegion,
  MotionCatalogEntry,
  MotionFamily,
  MotionLoopMode,
  MotionRootMode,
} from "@hachimi/contracts";
import type { AppLocale } from "@hachimi/i18n";

export const MOTION_FAMILIES: readonly MotionFamily[] = [
  "idle",
  "reaction",
  "gesture",
  "speech",
  "locomotion",
  "performance",
  "recovery",
  "unknown",
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

const familyLabels: Record<MotionFamily, readonly [string, string]> = {
  idle: ["待机", "Idle"],
  reaction: ["互动反应", "Reaction"],
  gesture: ["手势", "Gesture"],
  speech: ["说话动作", "Speaking"],
  locomotion: ["移动", "Locomotion"],
  performance: ["表演", "Performance"],
  recovery: ["恢复", "Recovery"],
  unknown: ["待分析", "Unclassified"],
};

const loopLabels: Record<MotionLoopMode, readonly [string, string]> = {
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

export function motionFamilyLabel(value: MotionFamily, locale: AppLocale): string {
  return localized(familyLabels[value], locale);
}

export function motionLoopLabel(value: MotionLoopMode, locale: AppLocale): string {
  return localized(loopLabels[value], locale);
}

export function motionRootLabel(value: MotionRootMode, locale: AppLocale): string {
  return localized(rootLabels[value], locale);
}

export function interactionRegionLabel(value: InteractionRegion, locale: AppLocale): string {
  return localized(regionLabels[value], locale);
}
