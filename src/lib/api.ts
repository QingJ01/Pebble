import { invoke } from "@tauri-apps/api/core";

export async function healthCheck(): Promise<string> {
  return invoke<string>("health_check");
}
