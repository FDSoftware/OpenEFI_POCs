/*
Esto por ahora tiene que ser simple como el carajo, asi que:

Tabla avance inicial 12x12 => despues la estiramo hasta 24x24

sin ninguna correccion por sensores ni lambda

usando sensors => map y rpm nomas

*/

#include "../include/ignition.hpp"

extern "C" {
#include <stdio.h>
#include <stdlib.h>

#include "../../sensors/utils/basic_electronics.h"
#include "trace.h"
}

table_data ignition::avc_tps_rpm;
bool ignition::loaded = false;
table_ref ignition_table = TABLES_IGNITION_TPS_SETTINGS;
bool ignition::fixed_mode = false;
bool ignition::error = false;

int32_t _AE = 0;

void ignition::interrupt() {
  if (!ignition::loaded || sensors::values._MAP <= 0 || fixed_mode || ignition::error) {
    _AE = ADVANCE_SAFE_VALUE;
    return;
  }

  /*
    int32_t dbg_map = sensors::values._MAP;
    int32_t dbg_map_v = get_input(4) * 1.534;

    dbg_map_v = dbg_map_v;
    dbg_map = dbg_map; */
  debug_printf("INIT IGNITION INTERRUPT \n");

  auto kpa_row = tables::col_to_row(ignition::avc_tps_rpm, 0);

  kpa_row.at(0) = 1;

#ifdef TESTING
  /*   debug_printf("KPA row values: \n");

    char row[200];
    for (auto table_x : kpa_row) {
      sprintf(row, "%s [%4ld]", row, table_x);
    }

    debug_printf("%s\n", row); */
  debug_printf("KPA row size: %ld \n", kpa_row.size());
#endif

  int32_t load_value = tables::find_nearest_neighbor(kpa_row, sensors::values._MAP);

  int32_t rpm_value = tables::find_nearest_neighbor(ignition::avc_tps_rpm.at(0), _RPM);

  debug_printf("LOAD var: %ld | RPM var: %ld \n", sensors::values._MAP, _RPM);

  debug_printf("LOAD index: %ld | RPM index: %ld \n", load_value, rpm_value);

  if (tables::on_bounds(ignition_table, load_value, rpm_value)) {
    _AE = avc_tps_rpm.at(load_value).at(rpm_value);
  }

  debug_printf("END IGNITION INTERRUPT \n");
}

void ignition::setup() {
  /* table_ref ignition_table = TABLES_IGNITION_TPS_SETTINGS; */
  ignition::avc_tps_rpm = tables::read_all(ignition_table);

  if (!tables::validate(ignition_table, ignition::avc_tps_rpm)) {
    _AE = ADVANCE_SAFE_VALUE;
    ignition::fixed_mode = true;
    ignition::error = true;
/*     tables::plot_table(ignition::avc_tps_rpm);
 */    trace_printf("Event: <IGNITION> Error loading TPS/RPM Table [INVALID_CRC]\r\n");

    // TODO: grabar DTC en memoria y/o entrar en modo de emergencia
  } else {
    ignition::loaded = true;
  }
}

void ignition::set_fixed_advance(int32_t adv) {
  _AE = adv;
  ignition::fixed_mode = true;
}

void ignition::disable_fixed_advance() {
  ignition::fixed_mode = false;
  ignition::interrupt();
}

/** Ejemplo tablita:
 * load(tps)/rpm
 * [  * ]  [550 ] [ 950] [1200] [1650] [2200] [2800] [3400] [3900] [4400] [4900]
 * [5400] [7200] [ 100]  (13.5) (13.0) (14.0) (14.0) (18.6) (24.0) (31.0) (33.0)
 * (33.2) (33.4) (33.6) (34.3) [ 90 ]  (13.8) (13.3) (14.0) (14.2) (17.4) (24.5)
 * (31.2) (33.3) (33.6) (33.8) (34.1) (34.9) [ 80 ]  (14.2) (13.6) (13.9) (14.4)
 * (17.8) (25.0) (31.5) (33.7) (34.0) (34.2) (34.5) (35.5) [ 70 ]  (14.5) (13.9)
 * (13.9) (14.6) (18.3) (25.5) (31.7) (34.0) (34.4) (34.7) (35.0) (36.1) [ 60 ]
 * (14.9) (14.2) (13.8) (14.8) (18.7) (25.9) (32.0) (34.4) (34.7) (35.1) (35.4)
 * (36.7) [ 50 ]  (15.3) (14.5) (13.8) (15.0) (19.1) (26.4) (32.2) (34.4) (35.1)
 * (35.5) (35.9) (37.3) [ 40 ]  (15.7) (14.8) (13.8) (15.2) (19.5) (26.9) (32.5)
 * (34.4) (35.5) (35.9) (36.4) (37.9) [ 30 ]  (16.1) (15.2) (14.4) (15.4) (19.9)
 * (27.4) (32.7) (34.4) (35.9) (36.4) (36.8) (38.5) [ 20 ]  (16.4) (15.5) (15.1)
 * (15.7) (20.4) (27.4) (34.9) (34.4) (36.3) (36.8) (37.3) (39.1) [ 15 ]  (16.6)
 * (15.7) (15.4) (15.8) (20.6) (28.3) (36.0) (34.4) (34.7) (37.0) (37.5) (39.4)
 * [ 10 ]  (16.8) (16.3) (15.7) (15.9) (20.8) (28.4) (36.0) (34.4) (34.7) (37.2)
 * (37.8) (39.7) [  5 ]  (17.0) (16.5) (16.0) (16.0) (21.0) (28.5) (36.0) (34.4)
 * (34.7) (37.4) (38.0) (40.0)
 */
/**
 *
 *  * [ 20 ]  164, 155, 151, 157, 204, 274, 349, 344, 363, 368, 373, 391,
 * [ 15 ]  166, 157, 154, 158, 206, 283, 360, 344, 347, 370, 375, 394,
 * [ 10 ]  168, 163, 157, 159, 208, 284, 360, 344, 347, 372, 378, 397,
 * [  5 ]  170, 165, 160, 160, 210, 285, 360, 344, 347, 374, 380, 400,
 */
/**
 [   0] [42000] [94000] [120000] [140000] [170000] [200000] [230000] [260000] [290000] [320000] [350000] [380000] [410000] [440000] [470000]
 [750000] [2500] [1300] [1000] [1610] [1930] [2260] [2590] [2910] [3240] [3570] [3679] [3679] [3679] [3679] [3679] [3679] [3679] [3000]
 [1300] [1000] [1590] [1910] [2230] [2550] [2880] [3200] [3520] [3629] [3629] [3629] [3629] [3629] [3629] [3629] [3500] [1300] [1000] [1570]
 [1889] [2200] [2520] [2840] [3160] [3479] [3590] [3590] [3590] [3590] [3590] [3590] [3590] [4000] [1300] [1000] [1550] [1860] [2180] [2490]
 [2810] [3120] [3440] [3540] [3540] [3540] [3540] [3540] [3540] [3540] [4500] [1300] [1000] [1530] [1839] [2150] [2460] [2770] [3080] [3390]
 [3500] [3500] [3500] [3500] [3500] [3500] [3500] [5000] [1300] [1000] [1510] [1810] [2120] [2430] [2730] [3040] [3350] [3450] [3450] [3450]
 [3450] [3450] [3450] [3450] [5500] [1300] [1000] [1490] [1789] [2090] [2400] [2700] [3000] [3300] [3410] [3410] [3410] [3410] [3410] [3410]
 [3410] [6000] [1300] [1000] [1470] [1770] [2070] [2360] [2660] [2960] [3260] [3360] [3360] [3360] [3360] [3360] [3360] [3360] [6600] [1300]
 [1000] [1440] [1739] [2030] [2330] [2620] [2910] [3210] [3310] [3310] [3310] [3310] [3310] [3310] [3310] [7100] [1300] [1000] [1430] [1720]
 [2000] [2290] [2580] [2870] [3160] [3260] [3260] [3260] [3260] [3260] [3260] [3260] [7600] [1300] [1000] [1410] [1689] [1980] [2260] [2550]
 [2830] [3120] [3220] [3220] [3220] [3220] [3220] [3220] [3220] [8100] [1300] [1000] [1390] [1670] [1950] [2230] [2510] [2800] [3080] [3170]
 [3170] [3170] [3170] [3170] [3170] [3170] [8600] [1300] [1000] [1370] [1639] [1920] [2200] [2480] [2760] [3030] [3130] [3130] [3130] [3130]
 [3130] [3130] [3130] [9100] [ 500] [1000] [1350] [1620] [1889] [2170] [2440] [2720] [2990] [3080] [3080] [3080] [3080] [3080] [3080] [3080]
 [9600] [ 500] [1000] [1330] [1600] [1870] [2140] [2410] [2680] [2950] [3040] [3040] [3040] [3040] [3040] [3040] [3040]
 [10100] [ 500] [1000] [1310] [1570] [1839] [2100] [2370] [2640] [2900] [2990] [2990] [2990] [2990] [2990] [2990] [2990]

 */