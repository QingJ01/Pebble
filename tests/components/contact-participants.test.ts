import { describe, expect, it } from "vitest";
import { uniqueContactParticipants, uniqueParticipants } from "@/components/contact-participants";

describe("uniqueParticipants", () => {
  it("deduplicates addresses case-insensitively and drops empty ones", () => {
    expect(uniqueParticipants([
      { name: "Alice", address: " alice@example.com " },
      { name: "Alice duplicate", address: "ALICE@example.com" },
      { name: "Missing", address: "" },
    ])).toEqual([{ name: "Alice", address: "alice@example.com" }]);
  });
});

describe("uniqueContactParticipants", () => {
  it("deduplicates From, To, and Cc addresses case-insensitively", () => {
    const participants = uniqueContactParticipants(
      { name: "Sender", address: " Sender@example.com " },
      [
        { name: "Sender duplicate", address: "sender@EXAMPLE.com" },
        { name: "Destination", address: "destination@example.com" },
      ],
      [
        { name: "Destination duplicate", address: "DESTINATION@example.com" },
        { name: "Copy", address: "copy@example.com" },
      ],
    );

    expect(participants).toEqual([
      { name: "Sender", address: "Sender@example.com" },
      { name: "Destination", address: "destination@example.com" },
      { name: "Copy", address: "copy@example.com" },
    ]);
  });

  it("drops participants with empty addresses", () => {
    expect(uniqueContactParticipants(
      { name: "Missing", address: " " },
      [{ name: "Valid", address: "valid@example.com" }],
      [],
    )).toEqual([{ name: "Valid", address: "valid@example.com" }]);
  });
});
