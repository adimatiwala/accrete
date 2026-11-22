"""Actor-critic network for the Accrete environment.

The policy scores every action slot with a shared head, so an option is
judged by what it is (its slot features and a learned embedding of its kind
and rank) in the context of what the agent knows (grid and vector encoders):

    context = MLP([CNN(grid), MLP(vector)])
    logit_k = MLP([context, slot_features_k, embedding_k])     for each slot k
    value   = Linear(context)

Masked slots get a logit of -1e9, so they are never sampled.
"""

from __future__ import annotations

import torch
from torch import nn
from torch.distributions import Categorical

from . import _core

MASKED_LOGIT = -1e9


def obs_to_tensors(obs: dict, device: torch.device | str = "cpu") -> dict[str, torch.Tensor]:
    """Convert a (batched or single) observation dict of NumPy arrays to tensors."""
    out = {}
    for k, v in obs.items():
        t = torch.as_tensor(v, device=device)
        out[k] = t.bool() if k == "action_mask" else t.float()
    return out


class ActorCritic(nn.Module):
    def __init__(self, hidden: int = 256, slot_embed: int = 16) -> None:
        super().__init__()
        c, side = _core.GRID_CHANNELS, _core.GRID_SIDE
        self.grid = nn.Sequential(
            nn.Conv2d(c, 32, 3, padding=1),
            nn.ReLU(),
            nn.Conv2d(32, 64, 3, stride=2, padding=1),
            nn.ReLU(),
            nn.Conv2d(64, 64, 3, stride=2, padding=1),
            nn.ReLU(),
            nn.Flatten(),
        )
        with torch.no_grad():
            grid_out = self.grid(torch.zeros(1, c, side, side)).shape[1]
        self.vector = nn.Sequential(nn.Linear(_core.VECTOR_SIZE, 128), nn.ReLU())
        self.context = nn.Sequential(nn.Linear(grid_out + 128, hidden), nn.ReLU())
        self.slot_embedding = nn.Embedding(_core.N_ACTIONS, slot_embed)
        self.slot_head = nn.Sequential(
            nn.Linear(hidden + _core.SLOT_FEATURES + slot_embed, 128),
            nn.ReLU(),
            nn.Linear(128, 1),
        )
        self.value_head = nn.Linear(hidden, 1)
        self.register_buffer("slot_ids", torch.arange(_core.N_ACTIONS), persistent=False)

    def forward(self, obs: dict[str, torch.Tensor]) -> tuple[torch.Tensor, torch.Tensor]:
        """Returns ``(masked logits [B, N], value [B])``."""
        grid, vector, slots, mask = obs["grid"], obs["vector"], obs["slots"], obs["action_mask"]
        if grid.dim() == 3:  # single observation
            grid, vector, slots, mask = grid[None], vector[None], slots[None], mask[None]
        ctx = self.context(torch.cat([self.grid(grid), self.vector(vector)], dim=-1))
        b, n = slots.shape[0], slots.shape[1]
        emb = self.slot_embedding(self.slot_ids).unsqueeze(0).expand(b, n, -1)
        per_slot = torch.cat([ctx.unsqueeze(1).expand(b, n, -1), slots, emb], dim=-1)
        logits = self.slot_head(per_slot).squeeze(-1)
        logits = logits.masked_fill(~mask, MASKED_LOGIT)
        return logits, self.value_head(ctx).squeeze(-1)

    def act(self, obs: dict[str, torch.Tensor], deterministic: bool = False):
        """Sample (or take the argmax of) the policy. Returns ``(action, log_prob, value)``."""
        logits, value = self(obs)
        dist = Categorical(logits=logits)
        action = logits.argmax(-1) if deterministic else dist.sample()
        return action, dist.log_prob(action), value

    def evaluate(self, obs: dict[str, torch.Tensor], actions: torch.Tensor):
        """Log-probabilities, entropy, and values for given actions."""
        logits, value = self(obs)
        dist = Categorical(logits=logits)
        return dist.log_prob(actions), dist.entropy(), value
