FROM rust:1.98-bookworm AS core
WORKDIR /build
COPY Cargo.toml Cargo.lock ./
COPY crates ./crates
RUN cargo build --release --locked -p kbc-node

FROM node:24-bookworm-slim AS adapter
WORKDIR /build
COPY package.json package-lock.json .npmrc tsconfig.json ./
RUN npm ci --ignore-scripts
COPY apps ./apps
RUN npx tsc
RUN npm prune --omit=dev --ignore-scripts

FROM node:24-bookworm-slim
RUN apt-get update && apt-get install -y --no-install-recommends ffmpeg && rm -rf /var/lib/apt/lists/*
ENV NODE_ENV=production
ARG NF_GIT_BRANCH
ARG NF_GIT_SHA
ENV BOT_BUILD_BRANCH=$NF_GIT_BRANCH
ENV BOT_BUILD_COMMIT=$NF_GIT_SHA
WORKDIR /app
COPY --from=adapter /build/node_modules ./node_modules
COPY --from=adapter /build/dist ./dist
COPY package.json ./
COPY content ./content
COPY data/search ./data/search
COPY scripts/search-snapshot.cjs ./scripts/search-snapshot.cjs
COPY --from=core /build/target/release/libkbc_node.so ./native/kbc_node.node
RUN mkdir storage && chown -R node:node /app
USER node
EXPOSE 3000
CMD ["node", "--env-file-if-exists=.env", "dist/main.js"]
