import { call, callRaw } from "./api";
import { authenticate } from "./auth";

export enum AccessRight {
  ManageObjects = "ManageObjects",
  GetObjectStats = "GetObjectStats",
  ManageBookmarks = "ManageBookmarks",
  ManageCollections = "ManageCollections",
  ManageSeries = "ManageSeries",
  ManageSubscriptions = "ManageSubscriptions",
}

/**
 * Output from API. Therefore, snake_case.
 */
export interface ObjectStats {
  size: number;
  created_at: string;
  last_touched_at: string;
  touches: number;
}

/**
 * Output from API. Therefore, snake_case.
 */
export interface Keypair {
  public: Array<number>;
  secret: Array<number>;
}

/**
 * Output from API. Therefore, snake_case.
 */
export interface SeriesOwner {
  name: string;
  keypair: Keypair;
  default_ttl: string;
  is_draft: boolean;
}

/**
 * Output from API. Therefore, snake_case.
 */
export interface Signed<T> {
  content: T;
}

export interface EditionContent {
  collection: string;
  timestamp: string;
  ttl: string;
}

/**
 * Output from API. Therefore, snake_case.
 */
export interface Edition {
  signed: Signed<EditionContent>;
  public_key: string;
  is_draft: boolean;
}

export enum SubscriptionKind {
  FullInventory = "FullInventory"
}

export interface Subscription {
  public_key: Array<number>,
  kind: SubscriptionKind, 
}

export class Samizdat {
  accessRights: Array<AccessRight>;
  isAuthenticated: boolean;
  kvstore: KVStore;

  constructor(accessRights: Array<AccessRight>) {
    this.accessRights = accessRights;
    this.isAuthenticated = false;
    this.kvstore = new KVStore();
  }

  /**
   * Explicitly authenticates this context to the current Samizdat node.
   * Authentication is done automatically when using this object.
   */
  async authenticate() {
    if (!this.isAuthenticated) {
      await authenticate(this.accessRights);
      this.isAuthenticated = true;
    }
  }

  async _ensureRights(necessaryRights: Array<AccessRight>) {
    for (const right of this.accessRights) {
      if (necessaryRights.includes(right)) {
        await this.authenticate();
        return;
      }
    }

    throw new Error(
      `Current scope needs any of ${necessaryRights} but only has ${this.accessRights}`
    );
  }

  async getObject(object: string) {
    const response = await call("GET", `/_objects/${object}`);
    return await response.blob();
  }

  async postObject(content: BodyInit) {
    await this._ensureRights([AccessRight.ManageObjects]);
    const response = await callRaw("POST", `/_objects`, content);
    return await response.text();
  }

  async deleteObject(object: string) {
    await this._ensureRights([AccessRight.ManageObjects]);
    const response = await call("DELETE", `/_objects/${object}`);
    return response.status < 300;
  }

  async reissue(object: string) {
    await this._ensureRights([AccessRight.ManageObjects]);
    const response = await call("POST", `/_objects/${object}/reissue`);
    return await response.text();
  }

  async bookmark(object: string) {
    await this._ensureRights([AccessRight.ManageBookmarks]);
    const response = await call("POST", `/_objects/${object}/bookmark`);
    return (await response.json())["Ok"] as null;
  }

  async isBookmarked(object: string) {
    await this._ensureRights([AccessRight.ManageBookmarks]);
    const response = await call("GET", `/_objects/${object}/bookmark`);
    return (await response.json())["Ok"] as boolean;
  }

  async unbookmark(object: string) {
    await this._ensureRights([AccessRight.ManageBookmarks]);
    const response = await call("DELETE", `/_objects/${object}/bookmark`);
    return (await response.json())["Ok"] as null;
  }

  async getStats(object: string) {
    await this._ensureRights([AccessRight.GetObjectStats]);
    const response = await call("DELETE", `/_objects/${object}/stats`);
    return (await response.json())["Ok"] as ObjectStats | null;
  }

  async getByteUsefulness(object: string) {
    await this._ensureRights([AccessRight.GetObjectStats]);
    const response = await call(
      "DELETE",
      `/_objects/${object}/stats/byte-usefulness`
    );
    return (await response.json())["Ok"] as number | null;
  }

  async postCollection(
    hashes: Array<[string, string]>,
    isDraft: boolean = false
  ) {
    await this._ensureRights([AccessRight.ManageCollections]);
    const response = await call("POST", `/_collections`, {
      hashes,
      is_draft: isDraft,
    });
    return await response.text();
  }

  async getItem(collection: string, path: string) {
    const response = await call("GET", `/_collections${collection}${path}`);
    return await response.blob();
  }

  // TODO: find a good ED25519 JS library to generate keypairs.
  // async postSeriesOwner() {
  //   const response = await call("GET", `/_seriesowners`);
  // }

  async getSeriesOwner(seriesOwner: string) {
    await this._ensureRights([AccessRight.ManageSeries]);
    const response = await call("GET", `/_seriesowner/${seriesOwner}`);
    return (await response.json())["Ok"] as SeriesOwner | null;
  }

  async deleteSeriesOwner(seriesOwner: string) {
    await this._ensureRights([AccessRight.ManageSeries]);
    const response = await call("DELETEs", `/_seriesowner/${seriesOwner}`);
    return (await response.json())["Ok"] as boolean;
  }

  async getSeriesOwners() {
    await this._ensureRights([AccessRight.ManageSeries]);
    const response = await call("GET", `/_seriesowner`);
    return (await response.json())["Ok"] as Array<SeriesOwner>;
  }

  async postEdition(
    seriesOwner: string,
    collection: string,
    ttl: string | null = null,
    noAnnounce: boolean = false
  ) {
    await this._ensureRights([AccessRight.ManageSeries]);
    const response = await call(
      "POST",
      `/_seriesowner/${seriesOwner}/editions`,
      {
        collection,
        ttl,
        no_announce: noAnnounce,
      }
    );
    return (await response.json())["Ok"] as Array<SeriesOwner>;
  }

  async getSeriesItem(seriesKey: string, path: string) {
    const response = await call("GET", `/_series${seriesKey}${path}`);
    return await response.blob();
  }

  async getSubscription(seriesKey: string) {
    const response = await call("GET", `/_subscriptions${seriesKey}`);
    return (await response.json())["Ok"] as Subscription | null;
  }

  async postSubscription(seriesKey: string, kind: SubscriptionKind = SubscriptionKind.FullInventory) {
    const response = await call("POST", `/_subscriptions`, { series_key: seriesKey, kind });
    return (await response.json())["Ok"] as string;
  }

  async deleteSubscription(seriesKey: string) {
    const response = await call("DELETE", `/_subscriptions${seriesKey}`);
    return (await response.json())["Ok"] as null;
  }
}

export class KVStore {
  constructor() {}

  async get(key: string) {
    const response = await call("GET", `/_kvstore/${key}`);
    return (await response.json())["Ok"] as string | null;
  }

  async put(key: string, value: string) {
    const response = await call("PUT", `/_kvstore/${key}`, { value });
    return (await response.json())["Ok"] as null;
  }

  async delete(key: string) {
    const response = await call("DELETE", `/_kvstore/${key}`);
    return (await response.json())["Ok"] as null;
  }

  async clear() {
    const response = await call("DELETE", `/_kvstore`);
    return (await response.json())["Ok"] as null;
  }
}

declare global {
  interface Window {
    Samizdat: any;
  }
}

window.Samizdat = Samizdat;
