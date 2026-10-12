import { existsSync, readdirSync } from "node:fs"
import { resolve } from "node:path"
import { defineConfig } from "vitest/config"

const root = __dirname

function vibeAliases(dir: string): Record<string, string> {
  const base = resolve(root, dir)
  if (!existsSync(base)) return {}
  return Object.fromEntries(
    readdirSync(base)
      .filter((name) => existsSync(resolve(base, name, "src", "index.ts")))
      .flatMap((name) => {
        const src = resolve(base, name, "src")
        return [
          [`vibe/${name}`, src],
          [`vibe/${name}/*`, resolve(src, "*")],
        ]
      }),
  )
}

const aliases = {
  ...vibeAliases("packages"),
  ...vibeAliases("tools/generators"),
}

function project(name: string, rootDir: string) {
  return {
    test: {
      name,
      root: resolve(root, rootDir),
      include: ["src/**/*.test.ts", "tests/**/*.test.ts"],
      exclude: ["node_modules", "dist", ".turbo"],
      globals: true,
      environment: "node",
      testTimeout: 20_000,
      hookTimeout: 10_000,
    },
    resolve: { alias: aliases },
  }
}

const packageDirs = readdirSync(resolve(root, "packages")).filter((name) =>
  existsSync(resolve(root, "packages", name, "src", "index.ts")),
)

export default defineConfig({
  test: {
    root: __dirname,
    projects: packageDirs.map((name) => project(`pkg:${name}`, `packages/${name}`)),
    coverage: {
      provider: "v8",
      reporter: ["text", "lcov", "html", "json-summary"],
      include: ["packages/*/src/**"],
      exclude: ["**/*.test.ts", "**/*.test-d.ts", "**/*.d.ts", "**/types/**"],
      thresholds: {
        statements: 80,
        branches: 70,
        functions: 75,
        lines: 80,
      },
    },
    reporters: ["default", "verbose"],
  },
  resolve: {
    alias: aliases,
  },
})
