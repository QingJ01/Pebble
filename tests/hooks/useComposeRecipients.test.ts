import { renderHook } from "@testing-library/react";
import { describe, expect, it } from "vitest";
import { useComposeRecipients } from "../../src/hooks/useComposeRecipients";
import type { Message } from "../../src/lib/ipc-types";

const original = {
  id: "m1", account_id: "a1", from_address: "dana@partner.example",
  to_list: [{ name: null, address: "me@work.example" }, { name: null, address: "lee@partner.example" }],
  cc_list: [{ name: null, address: "ops@partner.example" }],
} as unknown as Message;
const accounts = [{ id: "a1", email: "me@work.example" }] as never;

function recipients(composeMode: string, composePrefill: object | null) {
  return renderHook(() =>
    useComposeRecipients({ composeMode, composeReplyTo: original, accounts, activeAccountId: "a1", restoredDraft: null, composePrefill: composePrefill as never }),
  ).result.current;
}

describe("useComposeRecipients", () => {
  it("keeps the reply defaults when a prefill only brings a body", () => {
    expect(recipients("reply", { body: "Hi" }).to).toEqual(["dana@partner.example"]);
    const all = recipients("reply-all", { body: "Hi" });
    expect(all.to).toEqual(["dana@partner.example", "lee@partner.example"]);
    expect(all.cc).toEqual(["ops@partner.example"]);
  });

  it("uses recipients the prefill brings", () => {
    expect(recipients("reply", { to: ["x@y.example"], body: "Hi" }).to).toEqual(["x@y.example"]);
  });
});
