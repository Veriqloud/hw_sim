## Hardware simulator updates

The hardware now operate using 3 different sockets :
- one located at "/dev/xdma0_user" to receive the hardware controller commands
- one located at "/dev/xdma0_angles" to send the rotation angles
- one located at "/dev/xdma0_click_results" to send the measurements

### Input Commands

The input commands now changes to two commands :
- Start=0x27
- Stop=0x26

No response is expected.

### Angles and click results

The output of the previous hardware was u8. This output shall be splitted according to the following masks :
- secret = output & 0b10000000 != 0;
- basis = output & 0b01000000 != 0;
- measurement = output & 0b00000001 != 0;
