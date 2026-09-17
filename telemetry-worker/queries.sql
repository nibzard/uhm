-- blob positions are fixed by src/index.js:
-- blob1 release, blob2 os, blob3 arch, blob4 shell, blob5 mode,
-- blob6 route, blob7 decision, blob8 effect, blob9 proposal outcome,
-- blob10 execution outcome, blob11 feedback, blob12 latency, blob13 cache,
-- blob14 parent action acknowledgement, blob15 expansion outcome.
-- blob15 is absent in rows written before the expanded v2 shape; treat a
-- missing blob15 as the legacy value "none" so historical samples stay
-- meaningful in the same query.

-- Interactions by route; feedback_summary is deliberately excluded.
SELECT blob6 AS route, SUM(_sample_interval) AS interactions
FROM uhm_cli_v1
WHERE index1 = 'interaction_summary'
GROUP BY route
ORDER BY interactions DESC;

-- Proposal and execution evidence remain separate.
SELECT blob9 AS proposal_outcome, blob10 AS execution_outcome,
       SUM(_sample_interval) AS events
FROM uhm_cli_v1
WHERE index1 = 'interaction_summary'
GROUP BY proposal_outcome, execution_outcome
ORDER BY events DESC;

-- Explicit feedback does not count as another interaction.
SELECT blob11 AS feedback, SUM(_sample_interval) AS responses
FROM uhm_cli_v1
WHERE index1 = 'feedback_summary'
GROUP BY feedback;

-- Parent action state is unknown until the wrapper acknowledges it.
SELECT blob14 AS parent_action, SUM(_sample_interval) AS events
FROM uhm_cli_v1
WHERE index1 = 'interaction_summary'
GROUP BY parent_action;

-- Coarse subcommand-probe expansion outcome; historical rows without blob15
-- count as "none".
SELECT COALESCE(blob15, 'none') AS expansion_outcome,
       SUM(_sample_interval) AS events
FROM uhm_cli_v1
WHERE index1 = 'interaction_summary'
GROUP BY expansion_outcome;
