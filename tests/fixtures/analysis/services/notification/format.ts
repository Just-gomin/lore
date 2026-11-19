import type { Charge } from "@/services/payment/core/types.js";
import { charge } from "@core";
import missing from "@/services/none";

export function format(value: Charge): string {
  return `${value.amount}${charge}${missing}`;
}
