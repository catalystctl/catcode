/**
 * Reconcile a successful file save against the live editor buffer.
 *
 * The submitted snapshot, not the current buffer, becomes the saved baseline.
 * Text entered while the request was in flight therefore remains dirty.
 */
export function reconcileSavedSnapshot(
  submitted: string,
  current: string,
): { saved: string; dirty: boolean } {
  return { saved: submitted, dirty: current !== submitted };
}

/** Serialize async work while allowing a failed operation to be followed by another. */
export class SerializedTaskQueue {
  private tail: Promise<void> = Promise.resolve();
  private pending = 0;

  constructor(private readonly onPendingChange: (pending: number) => void = () => {}) {}

  enqueue(task: () => Promise<void>): Promise<void> {
    this.pending += 1;
    this.onPendingChange(this.pending);
    const operation = this.tail.catch(() => undefined).then(task);
    this.tail = operation.then(
      () => undefined,
      () => undefined,
    );
    return operation.finally(() => {
      this.pending = Math.max(0, this.pending - 1);
      this.onPendingChange(this.pending);
    });
  }
}
