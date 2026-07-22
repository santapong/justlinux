"""Generate the cell SRDF: 3 planning groups + the collision matrix.

The Allowed Collision Matrix disables checks for link pairs that can't
meaningfully collide:
  · Adjacent  — joined by a joint (their geometry always overlaps at the joint)
  · Never     — too far apart in the chain to ever reach each other
Everything NOT disabled stays CHECKED — crucially arm-vs-arm and arm-vs-car
part, so the planner keeps the robots from crashing into each other or the
workpiece. Written by reasoning about the chain (what the MoveIt Setup
Assistant computes by sampling)."""

ARMS = ["arm1_", "arm2_", "arm3_"]
LINKS = ["base_link", "link1", "link2", "link3", "link4", "link5", "tool0"]


def disables_for(p):
    out = []
    # adjacent (consecutive) links — connected by a joint
    for a, b in zip(LINKS, LINKS[1:]):
        out.append((p + a, p + b, "Adjacent"))
    # one-apart links near a joint can still overlap → disable (Never/Default)
    for i in range(len(LINKS) - 2):
        out.append((p + LINKS[i], p + LINKS[i + 2], "Never"))
    return out


def main():
    L = ['<?xml version="1.0"?>',
         '<!-- Auto-generated cell SRDF: groups + collision matrix (ACM). -->',
         '<robot name="weld_cell">']
    # one planning group per arm (the whole chain)
    for i, p in enumerate(ARMS, 1):
        L.append(f'  <group name="arm{i}">')
        L.append(f'    <chain base_link="{p}base_link" tip_link="{p}tool0"/>')
        L.append('  </group>')
        # a couple of named poses per group
        L.append(f'  <group_state name="home" group="arm{i}">')
        for j in range(1, 7):
            L.append(f'    <joint name="{p}joint{j}" value="0"/>')
        L.append('  </group_state>')
    # virtual joint: the cell is bolted to the world
    L.append('  <virtual_joint name="cell_base" type="fixed" '
             'parent_frame="world" child_link="world"/>')
    # ---- the collision matrix ----
    total = 0
    for p in ARMS:
        L.append(f'  <!-- {p} self-collisions -->')
        for a, b, reason in disables_for(p):
            L.append(f'  <disable_collisions link1="{a}" link2="{b}" '
                     f'reason="{reason}"/>')
            total += 1
    L.append('</robot>')
    print("\n".join(L))
    import sys
    print(f"<!-- {total} disabled pairs; arm-vs-arm and arm-vs-car_part "
          f"stay CHECKED -->", file=sys.stderr)


if __name__ == "__main__":
    main()
