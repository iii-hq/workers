# Stage 1: build
FROM node:22-slim AS build
WORKDIR /app
COPY package.json package-lock.json* ./
RUN npm install --no-audit --no-fund
COPY src/ src/
COPY tsconfig.json .
RUN npx tsc

# Stage 2: runtime
FROM node:22-slim
WORKDIR /app
COPY package.json package-lock.json* ./
RUN npm install --omit=dev --no-audit --no-fund
COPY --from=build /app/dist ./dist
ENV III_URL=ws://localhost:49134
CMD ["node", "/app/dist/index.js"]
