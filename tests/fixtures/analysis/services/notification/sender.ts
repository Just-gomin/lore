import * as core from "../payment/core";
import { alert as notify } from "./alert";

export default class Sender {
  send(): string {
    return notify(core.charge(1).amount);
  }
}

const helper = () => core.charge(2);

export { helper as run };
