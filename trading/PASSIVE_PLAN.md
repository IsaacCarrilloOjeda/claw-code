# The actually-passive portion

Ghost is an active/semi-systematic strategy. It can't carry a "passive income"
goal on its own. This document is the other 80% — the part that compounds
quietly while Ghost validates (or doesn't).

Do this once, in a weekend. Then forget about it.

## Allocation target

| Sleeve | % | Holds | Why |
|---|---|---|---|
| Broad market | 50% | VOO or VTI | S&P 500 / total market. Historical ~10% nominal. Tax-efficient. |
| T-bills | 20% | SGOV or BIL | ~4–5% yield, no duration risk, beats HYSA, liquid |
| Dividend growth (in Roth) | 10% | SCHD | Quality dividend payers. In a Roth, the dividends never get taxed. |
| Ghost sleeve | 20% | cash until validated, then paper, then live | Active strategy. Fun-money-with-a-thesis. |

Total passive share = 80%. If Ghost fails, you lose 20% of 20% = 4% of portfolio
max before shutting it down. If Ghost works, it outperforms and earns
reallocation.

## One-time setup

**Age note:** Since you're 15, accounts need to be custodial (UTMA/UGMA or custodial Roth) — a parent is the custodian until you turn 18 or 21 (state-dependent). This isn't a hoop to jump through; it's paperwork signed once.

1. **Custodial Roth IRA** — Fidelity Youth / Schwab custodial Roth. Requires reported **earned income** (W-2 or self-employment income filed on a tax return — RC Concrete freelance pay or KYNE invoices count if you actually file). Contribution limit 2026: $7,000/yr or your total earned income, whichever is lower. Inside a Roth, dividends and gains never get federally taxed again.
2. **Custodial taxable brokerage (UTMA/UGMA)** — same brokerage, opened by a parent. Holds VOO + SGOV. Money is legally yours at age of majority.
3. **Turn on DRIP** (dividend reinvestment) on every position.
4. **Automatic contributions** — fixed dollar amount biweekly or monthly. Dollar-cost-averaging beats retail market-timing by a mile.
5. **Don't look at it more than quarterly.** Rebalance once a year if any sleeve has drifted >5pp from target.

**Until you're 18:** Alpaca paper trading is fine (no age gate). Alpaca live requires 18. Plan = paper until 18, then live — by then the Ghost signal is either validated or proven dead, so you're not flipping to live trading on a hunch.

## Why these funds specifically

- **VOO** — Vanguard S&P 500 ETF. Expense ratio 0.03%. 500 largest US
  companies. The benchmark every active strategy tries to beat; most fail.
- **VTI** — Vanguard Total Market. Same idea, slightly broader (includes mid/small cap).
  Pick one, not both.
- **SGOV** — iShares 0-3 month T-Bill ETF. Expense ratio 0.09%. Yield tracks
  the Fed funds rate. Pays monthly. Zero credit risk. Beats HYSA in 2026.
- **SCHD** — Schwab US Dividend Equity. Expense ratio 0.06%. ~3.5% yield,
  dividend growers only. Inside a Roth, those dividends compound federally untaxed forever.

## What NOT to do

- No individual stock picking in the passive sleeve. That's what Ghost is for.
- No leveraged or inverse ETFs (TQQQ, SQQQ, etc). Decay kills them long-term.
- No crypto in this sleeve. If you want crypto, that's a third sleeve with its own thesis — don't call it passive.
- No JEPI/JEPQ (covered-call ETFs) in a taxable account — the distributions are taxed as ordinary income, which eats the yield advantage. They're fine in a Roth if you want monthly income.
- Don't check daily. Don't "move to cash" because CNBC panicked. Don't try to time re-entry. Dollar-cost-average through everything.

## Tracking

You can track this with the same Alpaca ledger — Alpaca supports taxable and
IRA accounts. Or Fidelity's built-in performance page, which is free and fine.
Don't build a custom dashboard until you actually have a use case; the passive
sleeve needs to be uninteresting on purpose.
