import { Container, getContainer } from "@cloudflare/containers";
import { cacheKey, cacheable, fromCache, storeInCache } from "./cache";

// Secrets are not part of the generated Env type. Database settings and sync
// tokens are forwarded verbatim into this retired container deployment; any
// configured endpoint must be reachable from the container's own network.
type ShellEnv = Env & {
  SURREALDB_ENDPOINT?: string;
  SURREALDB_NAMESPACE?: string;
  SURREALDB_DATABASE?: string;
  SURREALDB_USERNAME?: string;
  SURREALDB_PASSWORD?: string;
  SPIRE_SYNC_TOKEN?: string;
  FITNESS_SYNC_TOKEN?: string;
};

export class BenjispongeContainer extends Container<ShellEnv> {
  defaultPort = 8080;
  sleepAfter = "15m";

  constructor(ctx: BenjispongeContainer["ctx"], env: ShellEnv) {
    super(ctx, env);
    // envVars is the only channel into the container process, and it is read
    // at instance start — rotating a secret needs a container restart.
    // Unset secrets become empty strings, which the Rust side treats as
    // "closed" (auth) or "unconfigured" (database).
    this.envVars = {
      SITE_ORIGIN: "https://ben.soy",
      SURREALDB_ENDPOINT: env.SURREALDB_ENDPOINT ?? "",
      SURREALDB_NAMESPACE: env.SURREALDB_NAMESPACE ?? "",
      SURREALDB_DATABASE: env.SURREALDB_DATABASE ?? "",
      SURREALDB_USERNAME: env.SURREALDB_USERNAME ?? "",
      SURREALDB_PASSWORD: env.SURREALDB_PASSWORD ?? "",
      SPIRE_SYNC_TOKEN: env.SPIRE_SYNC_TOKEN ?? "",
      FITNESS_SYNC_TOKEN: env.FITNESS_SYNC_TOKEN ?? "",
    };
  }
}

export default {
  async fetch(request, env, ctx): Promise<Response> {
    const url = new URL(request.url);

    // Keep every public alias on the canonical origin so the planes page's
    // Host-derived QR URL has one origin.
    if (
      [
        "www.ben.soy",
        "benjisponge.com",
        "www.benjisponge.com",
        "benmberman.com",
        "www.benmberman.com",
      ].includes(url.hostname)
    ) {
      return Response.redirect(
        `https://ben.soy${url.pathname}${url.search}`,
        301,
      );
    }

    const container = getContainer(env.SITE_CONTAINER, "site");

    // API responses are never edge-cached, whatever their method — the
    // container marks them no-store, but don't even consult the cache.
    if (url.pathname.startsWith("/api/") || !cacheable(request)) {
      return container.fetch(request);
    }

    const key = cacheKey(url, env.RELEASE_ID);
    const hit = await fromCache(key);
    if (hit) return hit;

    const response = await container.fetch(request);
    if (!response.ok) return response; // 404s stay uncached — cheap and deterministic
    return storeInCache(ctx, key, response);
  },
} satisfies ExportedHandler<Env>;
