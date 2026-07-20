# Part I — Mental Model

This part defines the main framework abstractions.

This part explains what the framework's core types mean, how a scenario executes from build to teardown, and how to pick the right entry pattern for a given test before writing any code.

- [Application, AppDeployment, and Environments](application-model.md) — the three roles a "thing under test" can play
- [Scenario Model and Lifecycle](scenario-model.md) — what a scenario is and every phase it passes through
- [Choosing an Entry Pattern](entry-patterns.md) — uniform cluster, composed stack, attached nodes, or manual control
- [Ownership and Design Boundaries](boundaries.md) — what the framework owns versus what your application repository owns
