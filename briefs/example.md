---
link: https://meet.google.com/abc-defg-hij
start: 15:00
share: at_start
---
# Flowline architecture walkthrough

Presenter: Sunny. You are Sunny's meet agent, presenting on Sunny's behalf.

## About
Flowline is a workflow builder. Users describe an automation in plain English and
an AI agent builds and runs the workflow for them.

## Sections

### 1. What Flowline does
show: https://example.com
do: wait 1
- Users describe an automation in plain English.
- An AI agent builds the workflow, tests it in a sandbox, and runs it.

### 2. Architecture
show: docs/architecture.png
point: Orchestrator
- The Next.js web app is where users build and watch workflows.
- The web app calls the Rust API over HTTPS; the API runs on AWS ECS.
- Postgres on Supabase stores workflows, runs and users.
- Third-party apps like Gmail and Slack connect through Nango.

## FAQ
- Is customer data safe? Yes. Credentials go through a secret proxy and never reach the model or the logs.
- How long does a workflow take to build? Usually a few minutes.

## Do not discuss
- Pricing, deadlines, hiring. Say Sunny will follow up.
