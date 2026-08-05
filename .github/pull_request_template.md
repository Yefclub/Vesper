## What this changes

<!-- The problem, and what you did about it. Link the issue if there is one. -->

## How to check it

<!-- What a reviewer should run or click to see that it works. -->

## Verification

<!-- Paste the output. An assertion without evidence does not count. -->

```
npm run typecheck
npm run build
cd src-tauri && cargo test
```

## Checklist

- [ ] Every changed line traces back to what was asked for
- [ ] New user-facing strings exist in both `en` and `pt-BR`
- [ ] Nothing new reaches the network — or it does, and the pull request says so and why
- [ ] No schema change that drops or renames a column
