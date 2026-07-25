export class SecondaryMotionRuntime {
  private accumulator = 0;
  private readonly fixedStep = 1 / 120;

  update(deltaSeconds: number, step: (deltaSeconds: number) => void): void {
    if (!Number.isFinite(deltaSeconds) || deltaSeconds <= 0 || deltaSeconds > 0.25) {
      this.reset();
      return;
    }
    this.accumulator = Math.min(this.accumulator + deltaSeconds, this.fixedStep * 4);
    let substeps = 0;
    while (this.accumulator >= this.fixedStep && substeps < 4) {
      step(this.fixedStep);
      this.accumulator -= this.fixedStep;
      substeps += 1;
    }
  }

  reset(): void {
    this.accumulator = 0;
  }
}
