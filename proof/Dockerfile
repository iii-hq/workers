# Stage 1: build
FROM node:22-slim AS build
WORKDIR /app
COPY package.json ./
RUN npm install --no-audit --no-fund --ignore-scripts
COPY src/ src/
COPY tsconfig.json .
RUN npx tsc

# Stage 2: runtime — Playwright base ships chromium + every system dep
# preinstalled, so we skip the postinstall `playwright install` here.
FROM mcr.microsoft.com/playwright:v1.52.0-jammy
WORKDIR /app
ENV PLAYWRIGHT_SKIP_BROWSER_DOWNLOAD=1
COPY package.json ./
RUN npm install --no-audit --no-fund --omit=dev --ignore-scripts
COPY --from=build /app/dist ./dist
ENV III_URL=ws://localhost:49134
CMD ["node", "/app/dist/worker.js"]
