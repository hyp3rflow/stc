// In methods of an object literal with a contextual type, 'this' has the
// contextual type.

type Point = {
    x: number;
    y: number;
    z?: number;
    moveBy(dx: number, dy: number, dz?: number): void;
}

let p1: Point = {
    x: 10,
    y: 20,
    moveBy(dx, dy, dz) {
        this.x += dx;
        this.y += dy;
        if (this.z && dz) {
            this.z += dz;
        }
    }
};

let p2: Point | null = {
    x: 10,
    y: 20,
    moveBy(dx, dy, dz) {
        this.x += dx;
        this.y += dy;
        if (this.z && dz) {
            this.z += dz;
        }
    }
};

let p3: Point | undefined = {
    x: 10,
    y: 20,
    moveBy(dx, dy, dz) {
        this.x += dx;
        this.y += dy;
        if (this.z && dz) {
            this.z += dz;
        }
    }
};

let p4: Point | null | undefined = {
    x: 10,
    y: 20,
    moveBy(dx, dy, dz) {
        this.x += dx;
        this.y += dy;
        if (this.z && dz) {
            this.z += dz;
        }
    }
};

declare function f1(p: Point): void;

f1({
    x: 10,
    y: 20,
    moveBy(dx, dy, dz) {
        this.x += dx;
        this.y += dy;
        if (this.z && dz) {
            this.z += dz;
        }
    }
});

declare function f2(p: Point | null | undefined): void;

f2({
    x: 10,
    y: 20,
    moveBy(dx, dy, dz) {
        this.x += dx;
        this.y += dy;
        if (this.z && dz) {
            this.z += dz;
        }
    }
});