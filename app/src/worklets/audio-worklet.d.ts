// The AudioWorkletGlobalScope is not part of TypeScript's DOM lib, so declare
// the pieces these worklets use. Kept minimal and matching the Web Audio spec.

/** Sample rate of the AudioContext that loaded the worklet. */
declare const sampleRate: number;
/** Time of the currently rendered block, in seconds. */
declare const currentTime: number;
/** Frame index of the currently rendered block. */
declare const currentFrame: number;

declare abstract class AudioWorkletProcessor {
  readonly port: MessagePort;
  constructor(options?: AudioWorkletNodeOptions);
}

declare function registerProcessor(
  name: string,
  processorCtor: new (options?: AudioWorkletNodeOptions) => AudioWorkletProcessor,
): void;
