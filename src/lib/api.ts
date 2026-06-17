import { invoke } from "@tauri-apps/api/core";
import type { AppBootstrap, ContentItem, DiagnosePageRequest, DownloadStatusFilter, LoginStatus, PageDiagnosis, PagedContentItems, SyncEvent, SyncSession, SyncStartRequest, TagOption, UserProfile } from "../types";

function hasTauriRuntime() {
  if (typeof window === "undefined") {
    return false;
  }
  const candidate = window as typeof window & {
    __TAURI_INTERNALS__?: unknown;
    __TAURI__?: unknown;
  };
  return Boolean(candidate.__TAURI_INTERNALS__ || candidate.__TAURI__);
}

async function safeInvoke<T>(command: string, payload?: Record<string, unknown>): Promise<T> {
  if (typeof invoke !== "function" || !hasTauriRuntime()) {
    throw new Error("当前页面运行在浏览器预览模式，Tauri 后端不可用。请使用 `npm run tauri dev` 启动桌面端。");
  }
  return invoke<T>(command, payload);
}

export async function bootstrapApp(): Promise<AppBootstrap> {
  if (!hasTauriRuntime()) {
    return {
      dbPath: "预览模式不可用",
      dataDir: "预览模式不可用",
      downloadDir: "预览模式不可用",
      downloadThreads: 2,
      activeSessionIds: [],
      version: "preview",
      buildDate: "",
      buildTime: "",
      buildLabel: "preview",
      appTitle: "今日头条收藏/喜欢同步 preview",
    };
  }
  return safeInvoke("bootstrap_app");
}

export async function chooseDataDirectory(): Promise<AppBootstrap | null> {
  return safeInvoke("choose_data_directory");
}

export async function migrateDataDirectory(): Promise<AppBootstrap | null> {
  return safeInvoke("migrate_data_directory");
}

export async function setDownloadThreads(value: number): Promise<AppBootstrap> {
  return safeInvoke("set_download_threads", { value });
}

export async function startSync(request: SyncStartRequest): Promise<SyncSession> {
  return safeInvoke("start_sync", { request });
}

export async function stopSync(sessionId: string): Promise<void> {
  return safeInvoke("stop_sync", { sessionId });
}

export async function launchDebugChrome(): Promise<void> {
  return safeInvoke("launch_debug_chrome");
}

export async function diagnosePage(request: DiagnosePageRequest): Promise<PageDiagnosis> {
  return safeInvoke("diagnose_page", { request });
}

export async function checkToutiaoLogin(request: DiagnosePageRequest): Promise<LoginStatus> {
  return safeInvoke("check_toutiao_login", { request });
}

export async function listSessions(): Promise<SyncSession[]> {
  if (!hasTauriRuntime()) {
    return [];
  }
  return safeInvoke("list_sync_sessions");
}

export async function listSyncEvents(sessionId?: string): Promise<SyncEvent[]> {
  if (!hasTauriRuntime()) {
    return [];
  }
  return safeInvoke("list_sync_events", { sessionId });
}

export async function searchItems(query: string, source?: "favorites" | "likes"): Promise<ContentItem[]> {
  const result = await searchItemsWithType(query, source);
  return result.items;
}

export async function searchItemsWithType(
  query: string,
  source?: "favorites" | "likes",
  contentType?: "article" | "video",
  tagFilters?: string[],
  downloadStatus?: DownloadStatusFilter,
  page = 1,
  pageSize = 50,
): Promise<PagedContentItems> {
  if (!hasTauriRuntime()) {
    return { items: [], total: 0, page, pageSize };
  }
  return safeInvoke("search_items", { query, source, contentType, tagFilters, downloadStatus, page, pageSize });
}

export async function getUserProfile(): Promise<UserProfile | null> {
  if (!hasTauriRuntime()) {
    return null;
  }
  return safeInvoke("get_user_profile");
}

export async function listTags(source?: "favorites" | "likes"): Promise<TagOption[]> {
  if (!hasTauriRuntime()) {
    return [
      { name: "所有", count: 0 },
      { name: "视频", count: 0 },
      { name: "文章", count: 0 },
      { name: "已下载", count: 0 },
      { name: "未下载", count: 0 },
      { name: "IT", count: 0 },
      { name: "编程", count: 0 },
      { name: "运动", count: 0 },
      { name: "医学", count: 0 },
      { name: "文化", count: 0 },
      { name: "壮族", count: 0 },
      { name: "语言", count: 0 },
      { name: "汉族", count: 0 },
      { name: "基因", count: 0 },
    ];
  }
  return safeInvoke("list_tags", { source });
}

export async function addItemTag(itemId: number, tag: string): Promise<string[]> {
  return safeInvoke("add_item_tag", { itemId, tag });
}

export async function removeItemTag(itemId: number, tag: string): Promise<string[]> {
  return safeInvoke("remove_item_tag", { itemId, tag });
}

export async function deleteContentItem(itemId: number): Promise<void> {
  return safeInvoke("delete_content_item", { itemId });
}

export async function openDownloadDir(): Promise<void> {
  return safeInvoke("open_download_dir");
}

export async function openItemDir(itemId: number): Promise<void> {
  return safeInvoke("open_item_dir", { itemId });
}

export async function openItemFile(itemId: number, kind: "article" | "video" | "cover" | "raw"): Promise<void> {
  return safeInvoke("open_item_file", { itemId, kind });
}
