import { round } from "@acme/money";
import type { Charge } from "./types.js";

export function charge(amount: number): Charge {
  return { amount: round(amount) };
}
