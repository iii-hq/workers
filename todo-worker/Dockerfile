# Stage 1: build
FROM node:22-slim AS build
WORKDIR /app
COPY package.json package-lock.json ./
RUN npm ci
COPY src/ src/
COPY tsconfig.json .
RUN npx tsc

# Stage 2: runtime
FROM node:22-slim
WORKDIR /app
COPY package.json package-lock.json ./
RUN npm ci --omit=dev
COPY --from=build /app/dist ./dist
ENV III_URL=ws://localhost:49134
CMD ["node", "/app/dist/index.js"]
