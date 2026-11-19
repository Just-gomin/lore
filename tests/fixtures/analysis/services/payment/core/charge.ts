import type { Charge } from "./types.js";

export function charge(amount: number): Charge {
  return { amount };
}
