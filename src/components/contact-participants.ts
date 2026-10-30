export interface ContactParticipant {
  name?: string | null;
  address: string;
}

export function uniqueParticipants(participants: readonly ContactParticipant[]): ContactParticipant[] {
  const seen = new Set<string>();
  const unique: ContactParticipant[] = [];
  for (const participant of participants) {
    const address = participant.address.trim();
    const normalizedAddress = address.toLowerCase();
    if (!normalizedAddress || seen.has(normalizedAddress)) continue;
    seen.add(normalizedAddress);
    unique.push({ name: participant.name, address });
  }
  return unique;
}

export function uniqueContactParticipants(
  sender: ContactParticipant,
  to: readonly ContactParticipant[],
  cc: readonly ContactParticipant[],
): ContactParticipant[] {
  return uniqueParticipants([sender, ...to, ...cc]);
}
