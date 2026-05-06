# Tiny webhook→SQS shim. Built on the official Bun image so we can run the
# TypeScript source directly with no compile step. The whole image is ~95MB.
FROM oven/bun:1.1.30-slim

WORKDIR /app

# Install deps first to keep the layer cache hot across index.ts edits.
# `--frozen-lockfile` makes CI builds reproducible — bun.lock pins
# @aws-sdk/client-sqs to an exact resolved version.
COPY package.json bun.lock ./
RUN bun install --production --frozen-lockfile

COPY index.ts ./
COPY tsconfig.json ./

ENV PORT=8080
EXPOSE 8080

# Use Bun itself for the healthcheck — no curl/wget in slim images.
HEALTHCHECK --interval=2s --timeout=2s --start-period=2s --retries=15 \
  CMD bun -e "fetch('http://localhost:'+(process.env.PORT||8080)+'/healthz').then(r=>process.exit(r.ok?0:1)).catch(()=>process.exit(1))"

CMD ["bun", "run", "index.ts"]
