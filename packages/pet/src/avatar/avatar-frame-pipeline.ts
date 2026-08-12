export interface AvatarFrameStages {
  sampleAndCompose(): void;
  inertialize(): void;
  applyInteractionFeedback(): void;
  solveFootContactsAndIk(): void;
  applyFaceGazeAndLipSync(): void;
  updateSpringBones(): void;
}

/** The order is part of Runtime V5's contract and is covered by an integration test. */
export function runAvatarFramePipeline(stages: AvatarFrameStages): void {
  stages.sampleAndCompose();
  stages.inertialize();
  stages.applyInteractionFeedback();
  stages.solveFootContactsAndIk();
  stages.applyFaceGazeAndLipSync();
  stages.updateSpringBones();
}
