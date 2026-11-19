import { readFileSync } from "node:fs";
import { charge } from "../payment/core";
import "./alert.css";

const template = require("./missing-template");

export function alert(amount: number): string {
  return `${template}${charge(amount).amount}${readFileSync}`;
}
